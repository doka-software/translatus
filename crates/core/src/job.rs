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
    pub fn store_chapter(
        &self,
        chapter: &crate::document::Chapter,
        config_sig: &str,
    ) -> Result<()> {
        let conn = self.lock()?;
        let tx = conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO cache(key,value) VALUES(?1,?2)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            )?;
            for seg in &chapter.segments {
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
            store.store_chapter(&chapter, sig).unwrap();
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
