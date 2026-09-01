//! Per-job persistence: a single SQLite file holding a content-addressed
//! translation cache + chapter checkpoints. This is what makes BYO-key safe —
//! a rate-limit, timeout or Ctrl-C never re-spends tokens, because every
//! translated segment is cached by `hash(source + config signature)` and resume
//! simply re-fills from the cache.

use crate::document::Book;
use crate::error::{CoreError, Result};
use rusqlite::{Connection, OpenFlags};
use sha2::{Digest, Sha256};
use std::path::Path;

pub struct JobStore {
    conn: std::sync::Mutex<Connection>,
}

/// Content-addressed cache key: identical source under identical settings reuses
/// the same translation (dedupes repeated paragraphs too).
pub fn cache_key(source: &str, config_sig: &str) -> String {
    let mut h = Sha256::new();
    h.update(config_sig.as_bytes());
    h.update(b"\x00");
    h.update(source.as_bytes());
    format!("{:x}", h.finalize())
}

/// Cache key for one segment's ANNOTATION. Position-salted (href + block index),
/// unlike the translation key: identical paragraphs may legitimately carry a
/// note at one occurrence and not another after the review pass, and a purely
/// content-addressed key would make those occurrences overwrite each other's
/// review verdicts. Resume stays 0-token — href/block_index are deterministic
/// re-parses of the same file.
pub fn note_cache_key(href: &str, block_index: usize, source: &str, anno_sig: &str) -> String {
    let mut h = Sha256::new();
    h.update(anno_sig.as_bytes());
    h.update(b"\x00note\x00");
    h.update(href.as_bytes());
    h.update(b"\x00");
    h.update(block_index.to_string().as_bytes());
    h.update(b"\x00");
    h.update(source.as_bytes());
    format!("{:x}", h.finalize())
}

/// Encode one note decision as its cache value: a deliberate skip stays the
/// compact `""` (the historical convention), a real note serialises as JSON
/// `{"pos":…,"text":…}` so its placement (AN-014) survives the cache.
pub fn encode_note_value(note: &crate::document::Note) -> String {
    if note.is_skip() {
        String::new()
    } else {
        serde_json::to_string(note).unwrap_or_else(|_| note.text.clone())
    }
}

/// Decode a note cache value. Three shapes load: `""` (skip), the JSON object
/// form, and — for caches written before placement existed — a bare plain
/// string, which reads as an "after" note (the only placement back then).
pub fn decode_note_value(value: &str) -> crate::document::Note {
    if value.is_empty() {
        return crate::document::Note::skip();
    }
    serde_json::from_str::<crate::document::Note>(value)
        .unwrap_or_else(|_| crate::document::Note::after(value.to_string()))
}

/// Whether a table is present. Read-only probes may be pointed at a file that
/// is not (yet) one of ours, and "no such table" is an answer, not a fault.
fn table_exists(conn: &Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
        [name],
        |_| Ok(()),
    )
    .is_ok()
}

pub fn file_hash(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

impl JobStore {
    pub fn open(path: &Path) -> Result<Self> {
        if std::fs::symlink_metadata(path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            return Err(CoreError::Other(format!(
                "refusing to open job cache through a symlink: {}",
                path.display()
            )));
        }
        // SQLite's NOFOLLOW flag rejects a symlink in *any* path component.
        // Canonicalise the containing directory first so harmless platform
        // aliases such as macOS `/tmp -> /private/tmp` remain usable, while the
        // final cache filename is still protected by the explicit check above
        // and SQLite's own final-component NOFOLLOW open.
        // A bare relative filename has `Some("")` as its parent; treat that as
        // the current directory instead of failing to canonicalise "".
        let parent = match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            _ => Path::new("."),
        };
        let file_name = path.file_name().ok_or_else(|| {
            CoreError::Other(format!(
                "job cache path has no file name: {}",
                path.display()
            ))
        })?;
        let resolved_path = std::fs::canonicalize(parent)?.join(file_name);
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&resolved_path)
            {
                Ok(file) => drop(file),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let meta = std::fs::symlink_metadata(&resolved_path)?;
                    if meta.file_type().is_symlink() || !meta.file_type().is_file() {
                        return Err(CoreError::Other(format!(
                            "job cache is not a regular file: {}",
                            path.display()
                        )));
                    }
                    if meta.nlink() > 1 {
                        return Err(CoreError::Other(format!(
                            "job cache has multiple hard links; refusing: {}",
                            path.display()
                        )));
                    }
                    std::fs::set_permissions(
                        &resolved_path,
                        std::fs::Permissions::from_mode(0o600),
                    )?;
                }
                Err(e) => return Err(e.into()),
            }
        }
        let conn = Connection::open_with_flags(
            &resolved_path,
            OpenFlags::default() | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        // If the DB is locked by another connection, wait up to 10s then error
        // out — never block forever (which previously froze the whole translation).
        let _ = conn.busy_timeout(std::time::Duration::from_secs(10));
        // DELETE journaling avoids long-lived `-wal` / `-shm` sidecars that can
        // inherit a permissive umask and expose translated passages beside an
        // otherwise owner-only main DB.
        let _ = conn.pragma_update(None, "journal_mode", "DELETE");
        let _ = conn.pragma_update(None, "synchronous", "NORMAL");
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS meta    (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS cache   (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS chapters(idx INTEGER PRIMARY KEY, href TEXT, status TEXT NOT NULL);
            "#,
        )?;
        // Mutex makes the store `Sync` so it can be held across `.await` in the
        // async GUI-host commands (which require a `Send` future).
        Ok(Self {
            conn: std::sync::Mutex::new(conn),
        })
    }

    /// Lock the connection, turning a poisoned mutex (a thread panicked while
    /// holding it) into a normal error instead of cascading panics that would
    /// wedge every subsequent translation.
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| crate::error::CoreError::Other("job DB lock poisoned".into()))
    }

    /// Open an EXISTING job cache read-only, or `None` when there is no job at
    /// `path`.
    ///
    /// [`JobStore::open`] creates the file when it is missing, which is right
    /// for a run and wrong for a *probe*. A resume-aware book list asks "does
    /// this book have unfinished work?" about every book on screen, and
    /// answering that with `open()` would drop an empty `.etjob` next to every
    /// book the user merely looked at.
    pub fn open_readonly(path: &Path) -> Result<Option<Self>> {
        let meta = match std::fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        if meta.file_type().is_symlink() || !meta.file_type().is_file() {
            return Err(CoreError::Other(format!(
                "refusing to open job cache through a symlink: {}",
                path.display()
            )));
        }
        // Same reason as `open`: SQLite's NOFOLLOW rejects a symlink in any path
        // component, so canonicalise the directory and let the explicit check
        // above plus NOFOLLOW guard the final name.
        let parent = match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            _ => Path::new("."),
        };
        let file_name = path.file_name().ok_or_else(|| {
            CoreError::Other(format!(
                "job cache path has no file name: {}",
                path.display()
            ))
        })?;
        let resolved_path = std::fs::canonicalize(parent)?.join(file_name);
        let conn = Connection::open_with_flags(
            &resolved_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        // A probe must never sit behind a running translation's lock: the book
        // list would freeze for ten seconds per book.
        let _ = conn.busy_timeout(std::time::Duration::from_millis(200));
        Ok(Some(Self {
            conn: std::sync::Mutex::new(conn),
        }))
    }

    /// Chapter indices this job has already finished (`status = 'done'`).
    ///
    /// Tolerates a job file with no `chapters` table — an interrupted first
    /// write, or a file that is not one of ours — by reporting no progress
    /// rather than failing the screen that asked.
    pub fn done_chapters(&self) -> Result<Vec<usize>> {
        let conn = self.lock()?;
        if !table_exists(&conn, "chapters") {
            return Ok(Vec::new());
        }
        let mut stmt = conn.prepare("SELECT idx FROM chapters WHERE status='done' ORDER BY idx")?;
        let rows = stmt.query_map([], |r| r.get::<_, i64>(0))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?.max(0) as usize);
        }
        Ok(out)
    }

    /// How many chapters the last run through this job saw. `None` for jobs
    /// written before the count was recorded — a resume can then only say how
    /// many chapters are done, not out of how many.
    pub fn total_chapters(&self) -> Result<Option<usize>> {
        Ok(self
            .get_meta("total_chapters")?
            .and_then(|v| v.trim().parse::<usize>().ok()))
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.lock()?.execute(
            "INSERT INTO meta(key,value) VALUES(?1,?2)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            (key, value),
        )?;
        Ok(())
    }

    pub fn get_meta(&self, key: &str) -> Result<Option<String>> {
        let conn = self.lock()?;
        // `open()` always creates the table; `open_readonly()` may be handed a
        // file that never got that far.
        if !table_exists(&conn, "meta") {
            return Ok(None);
        }
        let mut stmt = conn.prepare("SELECT value FROM meta WHERE key=?1")?;
        let mut rows = stmt.query([key])?;
        Ok(rows.next()?.map(|r| r.get::<_, String>(0)).transpose()?)
    }

    /// The config signature this job was translated under (cache-only re-render
    /// must reuse it, or every segment misses the cache).
    pub fn config_sig(&self) -> Result<Option<String>> {
        self.get_meta("config_sig")
    }

    pub fn target_lang(&self) -> Result<Option<String>> {
        self.get_meta("target_lang")
    }

    pub fn cache_get(&self, key: &str) -> Result<Option<String>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare("SELECT value FROM cache WHERE key=?1")?;
        let mut rows = stmt.query([key])?;
        Ok(rows.next()?.map(|r| r.get::<_, String>(0)).transpose()?)
    }

    pub fn cache_put(&self, key: &str, value: &str) -> Result<()> {
        self.lock()?.execute(
            "INSERT INTO cache(key,value) VALUES(?1,?2)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            (key, value),
        )?;
        Ok(())
    }

    /// Persist a freshly-translated batch of `(source, target)` pairs in ONE
    /// transaction. Called per streamed batch so an interrupted chapter keeps
    /// its completed batches (resume re-fills them and only the remainder is
    /// re-translated) — without this, a mid-chapter stall/crash discards every
    /// batch since the last chapter checkpoint.
    pub fn cache_put_batch(&self, pairs: &[(String, String)], config_sig: &str) -> Result<()> {
        if pairs.is_empty() {
            return Ok(());
        }
        let conn = self.lock()?;
        let tx = conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO cache(key,value) VALUES(?1,?2)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            )?;
            for (source, target) in pairs {
                stmt.execute((&cache_key(source, config_sig), target))?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Persist a batch of pre-computed `(cache key, value)` pairs in ONE
    /// transaction. The annotation path builds position-salted keys itself
    /// (see `note_cache_key`), so it can't reuse `cache_put_batch`.
    pub fn cache_put_raw_batch(&self, pairs: &[(String, String)]) -> Result<()> {
        if pairs.is_empty() {
            return Ok(());
        }
        let conn = self.lock()?;
        let tx = conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO cache(key,value) VALUES(?1,?2)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            )?;
            for (key, value) in pairs {
                stmt.execute((key, value))?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Fill `note` for every segment whose annotation (including the deliberate
    /// empty-string "no note here") is already cached under `anno_sig`. Returns
    /// the number of segments restored — the annotation resume hit-count.
    pub fn prefill_notes_from_cache(&self, book: &mut Book, anno_sig: &str) -> Result<usize> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare_cached("SELECT value FROM cache WHERE key=?1")?;
        let mut hits = 0usize;
        for chapter in &mut book.chapters {
            for seg in &mut chapter.segments {
                if seg.note.is_some() {
                    continue;
                }
                let key = note_cache_key(&chapter.href, seg.block_index, &seg.source, anno_sig);
                let mut rows = stmt.query([&key])?;
                if let Some(r) = rows.next()? {
                    seg.note = Some(decode_note_value(&r.get::<_, String>(0)?));
                    hits += 1;
                }
            }
        }
        Ok(hits)
    }

    pub fn set_chapter_status(&self, idx: usize, href: &str, status: &str) -> Result<()> {
        self.lock()?.execute(
            "INSERT INTO chapters(idx,href,status) VALUES(?1,?2,?3)
             ON CONFLICT(idx) DO UPDATE SET status=excluded.status",
            (idx as i64, href, status),
        )?;
        Ok(())
    }

    /// Fill `target` for every segment already present in the cache. Returns the
    /// number of segments restored (the resume hit-count). Locks once and reuses
    /// one prepared statement — on a large book this is thousands of point reads,
    /// so per-call locking/prepare was pure overhead.
    pub fn prefill_from_cache(&self, book: &mut Book, config_sig: &str) -> Result<usize> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare_cached("SELECT value FROM cache WHERE key=?1")?;
        let mut hits = 0usize;
        for chapter in &mut book.chapters {
            for seg in &mut chapter.segments {
                if seg.target.is_some() {
                    continue;
                }
                let key = cache_key(&seg.source, config_sig);
                let mut rows = stmt.query([&key])?;
                if let Some(r) = rows.next()? {
                    seg.target = Some(r.get::<_, String>(0)?);
                    hits += 1;
                }
            }
        }
        Ok(hits)
    }

    /// Persist every translated segment of a chapter into the cache — in ONE
    /// transaction so a chapter checkpoint is a single commit/fsync instead of
    /// one per segment (thousands of fsyncs over a large book otherwise).
    /// Persist a finished chapter's translations. `failed_units` names the
    /// segments whose "translation" is only the source text, kept because the
    /// provider call failed; those are deliberately not written, so the next
    /// run retries them instead of restoring a failure as a success.
    pub fn store_chapter(
        &self,
        chapter: &crate::document::Chapter,
        config_sig: &str,
        failed_units: &std::collections::HashSet<u64>,
    ) -> Result<()> {
        let conn = self.lock()?;
        let tx = conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO cache(key,value) VALUES(?1,?2)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            )?;
            for seg in &chapter.segments {
                if failed_units.contains(&(seg.block_index as u64)) {
                    continue;
                }
                if let Some(t) = &seg.target {
                    let key = cache_key(&seg.source, config_sig);
                    stmt.execute((&key, t))?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod readonly_probe_tests {
    use super::*;

    fn tmpdir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("et-job-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A probe answers a question about a job; it must never bring one into
    /// existence. A resume-aware book list probes every book on screen, and
    /// `open()`'s create-on-miss behaviour would leave an .etjob beside each.
    #[test]
    fn open_readonly_never_creates_a_job() {
        let d = tmpdir("absent");
        let path = d.join("nothing.etjob");
        assert!(JobStore::open_readonly(&path).unwrap().is_none());
        assert!(!path.exists());
    }

    #[test]
    fn a_probe_reads_progress_without_writing() {
        let d = tmpdir("progress");
        let path = d.join("book.etjob");
        {
            let store = JobStore::open(&path).unwrap();
            store.set_meta("total_chapters", "12").unwrap();
            store.set_chapter_status(0, "c0.xhtml", "done").unwrap();
            store.set_chapter_status(1, "c1.xhtml", "done").unwrap();
            store
                .set_chapter_status(2, "c2.xhtml", "in_progress")
                .unwrap();
        }
        let before = std::fs::metadata(&path).unwrap().len();

        let probe = JobStore::open_readonly(&path)
            .unwrap()
            .expect("the job exists");
        assert_eq!(probe.done_chapters().unwrap(), vec![0, 1]);
        assert_eq!(probe.total_chapters().unwrap(), Some(12));
        // Read-only means read-only: a write through the probe is refused.
        assert!(probe.set_meta("config_sig", "nope").is_err());
        drop(probe);
        assert_eq!(std::fs::metadata(&path).unwrap().len(), before);
    }

    /// Jobs written before the chapter count was recorded still probe cleanly —
    /// they just cannot say "out of how many".
    #[test]
    fn an_older_job_reports_progress_without_a_total() {
        let d = tmpdir("legacy");
        let path = d.join("book.etjob");
        {
            let store = JobStore::open(&path).unwrap();
            store.set_chapter_status(0, "c0.xhtml", "done").unwrap();
        }
        let probe = JobStore::open_readonly(&path).unwrap().unwrap();
        assert_eq!(probe.done_chapters().unwrap(), vec![0]);
        assert_eq!(probe.total_chapters().unwrap(), None);
    }

    /// A file that is not one of ours reads as "no progress", not as an error
    /// that takes a screen down with it.
    #[test]
    fn a_foreign_sqlite_file_reads_as_no_progress() {
        let d = tmpdir("foreign");
        let path = d.join("other.etjob");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE unrelated(x INTEGER);")
                .unwrap();
        }
        let probe = JobStore::open_readonly(&path).unwrap().unwrap();
        assert!(probe.done_chapters().unwrap().is_empty());
        assert_eq!(probe.total_chapters().unwrap(), None);
        assert_eq!(probe.config_sig().unwrap(), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Book, Chapter, Format, Segment};
    use std::collections::BTreeMap;

    fn tmp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("et-jobtest-{}-{}.etjob", std::process::id(), name))
    }
    fn cleanup(p: &std::path::Path) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{}", p.display(), suffix));
        }
    }
    fn book_one(src: &str) -> Book {
        Book {
            format: Format::Epub,
            title: None,
            chapters: vec![Chapter {
                spine_index: 0,
                href: "ch1.xhtml".into(),
                title: None,
                segments: vec![Segment::new(0, src.into(), BTreeMap::new())],
                apparatus: false,
            }],
        }
    }

    // The headline paid guarantee: a finished segment is cached and a fresh run
    // restores it (resume for free), and a changed config signature correctly
    // misses (no stale reuse).
    #[test]
    fn cache_persists_and_resume_restores() {
        let path = tmp("resume");
        cleanup(&path);
        let sig = "sig-v1";
        {
            let store = JobStore::open(&path).unwrap();
            let mut chapter = book_one("Hello").chapters.remove(0);
            chapter.segments[0].target = Some("你好".into());
            store
                .store_chapter(&chapter, sig, &Default::default())
                .unwrap();
        }
        {
            let store = JobStore::open(&path).unwrap();
            let mut book = book_one("Hello");
            let hits = store.prefill_from_cache(&mut book, sig).unwrap();
            assert_eq!(hits, 1, "resume must restore the cached segment for free");
            assert_eq!(book.chapters[0].segments[0].target.as_deref(), Some("你好"));
        }
        {
            let store = JobStore::open(&path).unwrap();
            let mut book = book_one("Hello");
            let hits = store.prefill_from_cache(&mut book, "sig-v2").unwrap();
            assert_eq!(
                hits, 0,
                "a changed config signature must invalidate the cache"
            );
        }
        cleanup(&path);
    }

    // Resilience guarantee: a mid-chapter batch checkpoint survives so an
    // interrupted run resumes from the last completed batch, not the chapter
    // start. Mirrors the streaming-checkpoint path in translate::run.
    /// A provider failure leaves the segment's "translation" equal to its
    /// source. Caching that turns one transient 401 into a permanently
    /// English book: every later run reports `units_failed: 0`, restores the
    /// source from cache, and never calls the model again.
    #[test]
    fn a_failed_segment_is_not_cached_so_the_next_run_retries_it() {
        let path = tmp("failed-not-cached");
        cleanup(&path);
        let sig = "sig-v1";

        let mut chapter = book_one("Hello").chapters.remove(0);
        chapter
            .segments
            .push(Segment::new(1, "World".into(), BTreeMap::new()));
        // Segment 0 failed: its target is the source. Segment 1 translated.
        chapter.segments[0].target = Some("Hello".into());
        chapter.segments[1].target = Some("世界".into());
        let failed: std::collections::HashSet<u64> = [0u64].into_iter().collect();

        {
            let store = JobStore::open(&path).unwrap();
            store.store_chapter(&chapter, sig, &failed).unwrap();
        }
        {
            let store = JobStore::open(&path).unwrap();
            let mut book = book_one("Hello");
            book.chapters[0]
                .segments
                .push(Segment::new(1, "World".into(), BTreeMap::new()));
            store.prefill_from_cache(&mut book, sig).unwrap();
            assert_eq!(
                book.chapters[0].segments[0].target, None,
                "the failed segment must come back unset so the run retries it"
            );
            assert_eq!(
                book.chapters[0].segments[1].target.as_deref(),
                Some("世界"),
                "a real translation must still resume for free"
            );
        }
        cleanup(&path);
    }

    #[test]
    fn batch_checkpoint_resumes_mid_chapter() {
        let path = tmp("batch-ckpt");
        cleanup(&path);
        let sig = "sig-v1";
        {
            // Simulate a chapter that stalled after one batch landed (never
            // reaching the chapter-boundary store_chapter).
            let store = JobStore::open(&path).unwrap();
            store
                .cache_put_batch(
                    &[
                        ("Hello".into(), "你好".into()),
                        ("World".into(), "世界".into()),
                    ],
                    sig,
                )
                .unwrap();
        }
        {
            let store = JobStore::open(&path).unwrap();
            // A two-segment book whose first segment was in the stalled batch.
            let mut book = book_one("Hello");
            book.chapters[0]
                .segments
                .push(Segment::new(1, "World".into(), BTreeMap::new()));
            book.chapters[0]
                .segments
                .push(Segment::new(2, "Unseen".into(), BTreeMap::new()));
            let hits = store.prefill_from_cache(&mut book, sig).unwrap();
            assert_eq!(
                hits, 2,
                "both checkpointed batch segments must resume for free"
            );
            assert_eq!(book.chapters[0].segments[0].target.as_deref(), Some("你好"));
            assert_eq!(book.chapters[0].segments[1].target.as_deref(), Some("世界"));
            assert_eq!(
                book.chapters[0].segments[2].target, None,
                "untranslated stays pending"
            );
        }
        cleanup(&path);
    }

    // Note-cache value contract: skip = "", note = JSON with placement; and a
    // legacy plain-string value (pre-AN-014 caches) still restores — as an
    // "after" note — instead of erroring or re-billing.
    #[test]
    fn note_cache_value_roundtrip_and_legacy_compat() {
        use crate::document::{Note, NotePos};
        // roundtrip
        let n = Note::new(NotePos::Before, "鋪墊");
        assert_eq!(decode_note_value(&encode_note_value(&n)), n);
        let skip = Note::skip();
        assert_eq!(encode_note_value(&skip), "");
        assert!(decode_note_value("").is_skip());
        // legacy plain string → after-note
        let legacy = decode_note_value("十九世紀捕鯨業的背景");
        assert_eq!(legacy, Note::after("十九世紀捕鯨業的背景"));

        // and through the store: a legacy raw value prefills as an after-note
        let path = tmp("legacy-note");
        cleanup(&path);
        let store = JobStore::open(&path).unwrap();
        let sig = "anno-sig";
        let mut book = book_one("Hello");
        let key = note_cache_key("ch1.xhtml", 0, "Hello", sig);
        store.cache_put(&key, "舊版純字串眉批").unwrap();
        let hits = store.prefill_notes_from_cache(&mut book, sig).unwrap();
        assert_eq!(hits, 1);
        assert_eq!(
            book.chapters[0].segments[0].note,
            Some(Note::after("舊版純字串眉批"))
        );
        cleanup(&path);
    }

    #[test]
    fn meta_and_cache_upsert_roundtrip() {
        let path = tmp("meta");
        cleanup(&path);
        let store = JobStore::open(&path).unwrap();
        store.set_meta("config_sig", "abc").unwrap();
        assert_eq!(store.config_sig().unwrap().as_deref(), Some("abc"));
        store.cache_put("k", "v1").unwrap();
        store.cache_put("k", "v2").unwrap(); // upsert, not duplicate
        assert_eq!(store.cache_get("k").unwrap().as_deref(), Some("v2"));
        assert_eq!(store.cache_get("missing").unwrap(), None);
        cleanup(&path);
    }
}

#[cfg(all(test, unix))]
mod permission_tests {
    use super::JobStore;
    use std::os::unix::fs::PermissionsExt;

    /// The cache is plaintext book content. On a shared machine the ambient
    /// umask would otherwise decide who can read it.
    #[test]
    fn a_new_job_cache_is_owner_only() {
        let path = std::env::temp_dir().join("et_perm_test.etjob");
        let _ = std::fs::remove_file(&path);
        let _store = JobStore::open(&path).expect("open");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        let _ = std::fs::remove_file(&path);
        assert_eq!(mode, 0o600, "job cache must be owner-only, got {mode:o}");
    }

    #[test]
    fn existing_job_permissions_are_repaired() {
        let path = std::env::temp_dir().join("et_perm_repair_test.etjob");
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, []).expect("seed file");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .expect("set permissive mode");
        let _store = JobStore::open(&path).expect("open");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        let _ = std::fs::remove_file(&path);
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn job_cache_refuses_symlinks_without_touching_target() {
        use std::os::unix::fs::symlink;
        let dir = std::env::temp_dir().join(format!("et_job_link_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("test dir");
        let target = dir.join("victim.sqlite");
        let link = dir.join("book.etjob");
        std::fs::write(&target, "sentinel").expect("victim");
        symlink(&target, &link).expect("symlink");
        assert!(JobStore::open(&link).is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "sentinel");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
