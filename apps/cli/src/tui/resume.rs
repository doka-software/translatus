//! "This book is half translated" — the resume state a run would pick up,
//! made visible.
//!
//! Resume is implicit in this product: two runs that derive the same job path
//! share one cache, and the second never re-bills what the first finished.
//! That is excellent behaviour and completely invisible in a terminal, which
//! is what this module fixes. It reads a job cache without writing to it and
//! answers the two questions the interactive screens ask:
//!
//! * the book list — "does this book have unfinished work, and how much?"
//! * the confirm gate — "how much of it does *this* run still have to pay for?"
//!
//! **What resume is keyed on.** The job file is derived from the output path
//! (`book.繁體中文.epub` → `book.繁體中文.etjob`), so changing the target
//! language, or typing a different "Save to", points the run at a *different*
//! job and starts over. That is the engine's contract, not something this
//! module can paper over, so the screens say it out loud instead: the confirm
//! gate probes the default job as well, and warns when a custom "Save to" is
//! about to walk away from finished work.
//!
//! **What voids a resume.** The cached translations are keyed by
//! `cache_signature()` (target language, depth, model, provider) and the
//! cached notes by `annotation_signature()`. Switching model or depth
//! therefore re-translates the whole book. [`Resume::reuses`] is the check,
//! and the gate says so rather than quietly quoting for a fresh run.
//!
//! **Cost.** The book list can hold dozens of books, so a probe is a `stat`
//! first and opens SQLite only for the books that actually have a job file.
//! Nothing here parses a book.

use et_core::job::JobStore;
use std::path::{Path, PathBuf};

/// What a job cache says about work already done on a book.
pub struct Resume {
    /// Chapter indices already finished.
    pub done: Vec<usize>,
    /// How many chapters the last run through this job saw. `None` for jobs
    /// written before the engine recorded it — then we can say how many
    /// chapters are done but not out of how many.
    pub total: Option<usize>,
    /// The translation signature the cached chapters were written under.
    pub config_sig: Option<String>,
    /// The annotation signature the cached notes were written under.
    pub anno_sig: Option<String>,
}

impl Resume {
    /// Read the job at `job_path`, or `None` when there is no job there.
    ///
    /// Every failure reads as "no resume". A probe decorates a screen; it must
    /// never be the reason the user cannot pick a book.
    pub fn probe(job_path: &Path) -> Option<Resume> {
        let store = JobStore::open_readonly(job_path).ok()??;
        let done = store.done_chapters().ok()?;
        Some(Resume {
            done,
            total: store.total_chapters().ok().flatten(),
            config_sig: store.config_sig().ok().flatten(),
            anno_sig: store.get_meta("anno_sig").ok().flatten(),
        })
    }

    /// A resume worth telling the user about: a job that finished at least one
    /// chapter and has not finished them all.
    pub fn unfinished(&self) -> bool {
        !self.done.is_empty() && self.total.is_none_or(|t| self.done.len() < t)
    }

    pub fn done_count(&self) -> usize {
        self.done.len()
    }

    /// Whether the cached work survives the settings this run will use.
    ///
    /// `want_config` is the run's `cache_signature()` (`None` for an
    /// annotate-only run, which translates nothing); `want_anno` its
    /// `annotation_signature()` (`None` when notes are off). A mismatch on
    /// either means the cache misses and the book runs again from the start —
    /// so a mismatch is a hard no, not a partial yes.
    pub fn reuses(&self, want_config: Option<&str>, want_anno: Option<&str>) -> bool {
        if let Some(want) = want_config {
            if self.config_sig.as_deref() != Some(want) {
                return false;
            }
        }
        if let Some(want) = want_anno {
            if self.anno_sig.as_deref() != Some(want) {
                return false;
            }
        }
        true
    }
}

/// The job cache a run with these choices will use.
///
/// Deliberately routed through the same helpers `run_translate` /
/// `run_annotate` use, so the path a screen probes is by construction the path
/// the run opens.
pub fn job_path(input: &Path, to: &str, translate: bool, output: Option<&Path>) -> PathBuf {
    let out = match output {
        Some(p) => p.to_path_buf(),
        None if translate => crate::default_output(input, to),
        None => crate::default_annotate_output(input),
    };
    crate::job_path_for(&out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use et_core::job::JobStore;

    fn tmpdir(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("translatus-resume-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// The book list probes every book on screen. A book that was never run
    /// must come back as "no resume" — and, critically, must not have gained
    /// an .etjob file just for being listed.
    #[test]
    fn probing_a_book_with_no_job_creates_nothing() {
        let d = tmpdir("nojob");
        let job = d.join("book.繁體中文.etjob");
        assert!(Resume::probe(&job).is_none());
        assert!(!job.exists(), "a probe must not create a job cache");
    }

    #[test]
    fn a_half_finished_job_reports_its_progress_and_signature() {
        let d = tmpdir("half");
        let job = d.join("book.繁體中文.etjob");
        {
            let store = JobStore::open(&job).unwrap();
            store.set_meta("total_chapters", "12").unwrap();
            store.set_meta("config_sig", "sig-a").unwrap();
            for i in 0..8 {
                store
                    .set_chapter_status(i, &format!("c{i}.xhtml"), "done")
                    .unwrap();
            }
            store
                .set_chapter_status(8, "c8.xhtml", "in_progress")
                .unwrap();
        }
        let r = Resume::probe(&job).expect("a job exists here");
        assert_eq!(r.done_count(), 8);
        assert_eq!(r.total, Some(12));
        assert!(r.unfinished());
        assert_eq!(r.done, vec![0, 1, 2, 3, 4, 5, 6, 7]);
        assert!(!r.done.contains(&8), "the interrupted chapter is not done");

        // The billing contract: same settings resume, changed settings do not.
        assert!(r.reuses(Some("sig-a"), None));
        assert!(
            !r.reuses(Some("sig-b"), None),
            "a new model must void the resume"
        );
        // Notes were never written here, so any annotation signature misses.
        assert!(!r.reuses(Some("sig-a"), Some("anno-1")));
    }

    /// A finished book is not "unfinished work" — the list must not nag about
    /// a job whose every chapter is done.
    #[test]
    fn a_finished_job_is_not_advertised_as_unfinished() {
        let d = tmpdir("finished");
        let job = d.join("book.繁體中文.etjob");
        {
            let store = JobStore::open(&job).unwrap();
            store.set_meta("total_chapters", "3").unwrap();
            for i in 0..3 {
                store
                    .set_chapter_status(i, &format!("c{i}.xhtml"), "done")
                    .unwrap();
            }
        }
        let r = Resume::probe(&job).unwrap();
        assert_eq!(r.done_count(), 3);
        assert!(!r.unfinished());
    }

    /// The output filename decides the job filename. This is the trap the
    /// screens have to warn about, so pin the derivation that makes it true.
    #[test]
    fn the_job_path_follows_the_output_path() {
        let input = Path::new("/books/Walden.epub");
        let default = job_path(input, "繁體中文", true, None);
        assert_eq!(default, Path::new("/books/Walden.繁體中文.etjob"));

        // A different target language is a different job — resume does not
        // carry across it.
        assert_ne!(default, job_path(input, "日本語", true, None));

        // A custom "Save to" moves the job with it.
        let custom = job_path(input, "繁體中文", true, Some(Path::new("/out/mine.epub")));
        assert_eq!(custom, Path::new("/out/mine.etjob"));

        // Annotate-only has its own default output, and so its own job.
        assert_eq!(
            job_path(input, "繁體中文", false, None),
            Path::new("/books/Walden.annotated.etjob")
        );
    }
}
