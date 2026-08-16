//! Where the CLI's settings live.
//!
//! This is deliberate rather than incidental. `et_core::settings::Settings` is
//! already the schema of the app's `settings.json`, and `et_core::secrets`
//! already writes to the same keychain service the app reads. Pointing the CLI
//! at the same locations means a key saved in one is present in the other, and
//! a book configured in one comes back configured in the other. Two stores
//! would have meant two sources of truth for the same question.

use et_core::settings::{BookAnnotationSettings, Settings};
use std::path::{Path, PathBuf};

/// The app's config directory, per platform convention.
///
/// Uses the platform app-config directory for the identifier in
/// the engine's config-dir contract; if that identifier ever changes, this has to
/// move with it or the two surfaces silently stop sharing.
const APP_ID: &str = "translatus";

pub fn config_dir() -> Option<PathBuf> {
    // An explicit override, honoured before anything else. Tests and CI must be
    // able to run the interactive flow without writing to the settings file a
    // a real person relies on — and a user who wants a throwaway
    // profile should not have to move their real one out of the way.
    if let Some(d) = std::env::var_os("TRANSLATUS_CONFIG_DIR") {
        if !d.is_empty() {
            return Some(PathBuf::from(d));
        }
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|h| h.join("Library/Application Support").join(APP_ID))
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|h| h.join(APP_ID))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|h| h.join(APP_ID))
    }
}

pub fn settings_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("settings.json"))
}

/// Load settings, falling back to defaults when there is no file yet.
pub fn load() -> Settings {
    match settings_path() {
        Some(p) => Settings::load(&p),
        None => Settings::default(),
    }
}

/// Persist. Failure is reported to the caller rather than swallowed: a setting
/// that silently fails to save is worse than one that refuses to.
pub fn save(s: &Settings) -> anyhow::Result<()> {
    let Some(p) = settings_path() else {
        anyhow::bail!("cannot determine a config directory on this platform");
    };
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    s.save(&p)?;
    Ok(())
}

/// This book's remembered setup, prefilled from the global defaults when the
/// book has not been seen before.
pub fn book_settings(s: &Settings, book: &Path) -> BookAnnotationSettings {
    let key = book
        .canonicalize()
        .unwrap_or_else(|_| book.to_path_buf())
        .to_string_lossy()
        .to_string();
    s.annotations.books.get(&key).cloned().unwrap_or({
        BookAnnotationSettings {
            translate: true,
            annotate: false,
            reader_profile: s.annotations.reader_profile.clone(),
            level: s.annotations.level.clone(),
            anchors: s.annotations.anchors.clone(),
            voice: s.annotations.voice.clone(),
            presets: s.annotations.presets.clone(),
            density: s.annotations.density.clone(),
            updated_at: 0,
        }
    })
}

/// Remember this book's setup. Oldest entries are pruned so the map cannot
/// grow without bound, matching the app's cap.
pub fn remember_book(s: &mut Settings, book: &Path, mut b: BookAnnotationSettings) {
    const MAX_BOOKS: usize = 100;
    let key = book
        .canonicalize()
        .unwrap_or_else(|_| book.to_path_buf())
        .to_string_lossy()
        .to_string();
    b.updated_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    s.annotations.books.insert(key, b);
    while s.annotations.books.len() > MAX_BOOKS {
        if let Some(oldest) = s
            .annotations
            .books
            .iter()
            .min_by_key(|(_, v)| v.updated_at)
            .map(|(k, _)| k.clone())
        {
            s.annotations.books.remove(&oldest);
        } else {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cli_points_at_the_same_file_the_app_writes() {
        // Guard against the override leaking in from the environment.
        std::env::remove_var("TRANSLATUS_CONFIG_DIR");
        let p = settings_path().expect("a config path on this platform");
        assert!(
            p.ends_with("settings.json"),
            "must be the app's file name: {p:?}"
        );
        assert!(
            p.to_string_lossy().contains(APP_ID),
            "must sit under the app's identifier, or the two surfaces stop \
             sharing settings: {p:?}"
        );
    }

    #[test]
    fn an_unseen_book_inherits_the_global_defaults() {
        let mut s = Settings::default();
        s.annotations.reader_profile = "a curious reader".into();
        s.annotations.presets = vec!["terms".into(), "history".into()];
        s.annotations.density = "rich".into();

        let b = book_settings(&s, Path::new("/tmp/never-opened.epub"));
        assert_eq!(b.reader_profile, "a curious reader");
        assert_eq!(b.presets, vec!["terms".to_string(), "history".to_string()]);
        assert_eq!(b.density, "rich");
        assert!(b.translate, "translate defaults on");
        assert!(!b.annotate, "annotate defaults off");
    }

    #[test]
    fn a_remembered_book_comes_back_with_its_own_answers() {
        let mut s = Settings::default();
        let book = Path::new("/tmp/some-book.epub");
        remember_book(
            &mut s,
            book,
            BookAnnotationSettings {
                translate: false,
                annotate: true,
                reader_profile: "for this book only".into(),
                presets: vec!["culture".into()],
                density: "sparse".into(),
                ..BookAnnotationSettings::default()
            },
        );
        let b = book_settings(&s, book);
        assert_eq!(b.reader_profile, "for this book only");
        assert!(!b.translate);
        assert!(b.annotate);
        assert_eq!(b.density, "sparse");
        // The globals are untouched — per-book edits stay per-book.
        assert_eq!(s.annotations.reader_profile, "");
    }

    #[test]
    fn the_per_book_map_cannot_grow_without_bound() {
        let mut s = Settings::default();
        for i in 0..130 {
            let b = BookAnnotationSettings {
                reader_profile: format!("book {i}"),
                ..BookAnnotationSettings::default()
            };
            remember_book(&mut s, Path::new(&format!("/tmp/b{i}.epub")), b);
        }
        assert!(
            s.annotations.books.len() <= 100,
            "expected pruning, got {}",
            s.annotations.books.len()
        );
    }
}

// ---------------------------------------------------------------------------
// Keychain, off the paint path
// ---------------------------------------------------------------------------

use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Mutex, OnceLock};

enum Hint {
    Pending(Receiver<Option<String>>),
    Ready(Option<String>),
}

/// Whether a key is saved, without ever blocking the screen.
///
/// On macOS the first read of a keychain entry written by a *different* binary
/// raises a modal system prompt. Another host may have written these entries, so the
/// CLI reading them is exactly that case. Doing it inline while building the
/// Settings screen froze the redraw until the user found and answered a dialog
/// they had no reason to expect — the terminal simply looked hung.
///
/// So the read happens once per session on a worker thread. The screen renders
/// with whatever is known, and since every keystroke repaints, the answer
/// appears as soon as it arrives.
pub fn key_hint_nonblocking(provider: &str) -> Option<String> {
    static CACHE: OnceLock<Mutex<Hint>> = OnceLock::new();
    let cell = CACHE.get_or_init(|| {
        let (tx, rx) = mpsc::channel();
        let provider = provider.to_string();
        std::thread::spawn(move || {
            let v = et_core::secrets::key_hint(&provider).ok().flatten();
            let _ = tx.send(v);
        });
        Mutex::new(Hint::Pending(rx))
    });
    let Ok(mut guard) = cell.lock() else {
        return None;
    };
    if let Hint::Pending(rx) = &*guard {
        match rx.try_recv() {
            Ok(v) => *guard = Hint::Ready(v),
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => *guard = Hint::Ready(None),
        }
    }
    match &*guard {
        Hint::Ready(v) => v.clone(),
        Hint::Pending(_) => None,
    }
}

#[cfg(test)]
mod override_tests {
    use super::*;

    /// Both tests mutate the same process-wide env var; without this lock the
    /// default parallel test runner makes them flaky against each other.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Without this, running the interactive flow in a test writes to the
    /// settings file.
    #[test]
    fn an_explicit_config_dir_wins() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("TRANSLATUS_CONFIG_DIR", "/tmp/et-override-test");
        let p = settings_path().expect("a path");
        std::env::remove_var("TRANSLATUS_CONFIG_DIR");
        assert_eq!(p, PathBuf::from("/tmp/et-override-test/settings.json"));
    }

    #[test]
    fn an_empty_override_falls_back_to_the_shared_location() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("TRANSLATUS_CONFIG_DIR", "");
        let p = settings_path().expect("a path");
        std::env::remove_var("TRANSLATUS_CONFIG_DIR");
        assert!(
            p.to_string_lossy().contains(APP_ID),
            "an empty override must not strand the settings somewhere else: {p:?}"
        );
    }
}
