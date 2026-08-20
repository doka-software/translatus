//! Persisted, non-secret settings shared by every surface of the engine.
//! Lives as a single JSON file in the app config dir. The API key is NOT here —
//! it lives in the OS keychain (see [`crate::secrets`]).

use crate::error::{CoreError, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

// Every field carries `#[serde(default …)]`: settings.json is a user-visible
// file that gets hand-edited and machine-pre-seeded. Without per-field
// defaults, ONE missing key used to fail the whole-struct parse and
// `Settings::load` silently fell back to factory defaults (provider "mock",
// model "mock") — an external pre-write of provider/model then never reached
// the UI. Partial files now degrade per-field instead of all-or-nothing.

fn default_target_lang() -> String {
    "繁體中文".into()
}
fn default_mode() -> String {
    "sentence".into()
}
fn default_output() -> String {
    "bilingual".into()
}
fn default_provider() -> String {
    "mock".into()
}
fn default_model() -> String {
    "mock".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralSettings {
    #[serde(default = "default_target_lang")]
    pub default_target_lang: String,
    /// "sentence" | "expert"
    #[serde(default = "default_mode")]
    pub default_mode: String,
    /// "replace" | "bilingual"
    #[serde(default = "default_output")]
    pub output: String,
    /// Where the interactive session looks for books, in addition to the
    /// directory it was launched from. `None` = only the launch directory.
    #[serde(default)]
    pub books_dir: Option<String>,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            default_target_lang: default_target_lang(),
            default_mode: default_mode(),
            // Bilingual interleave (source para / target para) is the product
            // default — locked by user decision 2026-06-12.
            output: default_output(),
            books_dir: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiSettings {
    #[serde(default = "default_provider")]
    pub provider: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default)]
    pub base_url: Option<String>,
    /// Which provider the saved `model` belongs to (the GUI's cross-provider
    /// leftover guard). Persisted so an app restart doesn't forget it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,
    /// Which model source the interactive session's picker was on
    /// ("subscription" / "api key" / "ollama"). `provider` alone cannot encode
    /// this: subscription and api-key mode are both `openai` to the engine.
    /// Absent in settings written by older versions; hosts fall back to
    /// inference and must not error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

impl Default for ApiSettings {
    fn default() -> Self {
        Self {
            provider: default_provider(),
            model: default_model(),
            base_url: None,
            model_provider: None,
            source: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModePrompt {
    /// None = use the engine default style for this mode.
    #[serde(default)]
    pub style: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Prompts {
    #[serde(default)]
    pub sentence: ModePrompt,
    #[serde(default)]
    pub expert: ModePrompt,
}

/// Annotation (眉批) preferences.
///
/// Two layers (user decision 2026-07-19, per-book onboarding UX):
/// - The flat fields are the user's GLOBAL DEFAULTS — what a NEWLY opened book
///   prefills. They are only rewritten when the user ticks 「存成我的預設」.
/// - `books` remembers each book's own setup (per-book override), keyed by the
///   book's absolute path, so re-opening the same book restores exactly what
///   the user configured for it. Hosts prune the oldest entries.
///
/// Nothing here is part of any cache signature until a run starts; see
/// `TranslateConfig::annotation_signature`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotationSettings {
    #[serde(default)]
    pub reader_profile: String,
    /// Default explanation level: "beginner" | "general" | "insider".
    #[serde(default = "default_level")]
    pub level: String,
    /// Default cognitive anchors (認知錨) — reader-level, not book-level.
    #[serde(default)]
    pub anchors: Vec<String>,
    /// Default voice register: "study" | "companion".
    #[serde(default = "default_voice")]
    pub voice: String,
    /// Saved note STYLE paragraph (眉批風格). `None` = engine default
    /// (`annotate::prompt::DEFAULT_NOTE_STYLE`). Feeds
    /// `AnnotationConfig::style`; part of the annotation signature only.
    #[serde(default)]
    pub note_style: Option<String>,
    /// Default preset help-angle ids (see `annotate::prompt::PRESETS`).
    #[serde(default)]
    pub presets: Vec<String>,
    /// Default density: "sparse" | "medium" | "rich".
    #[serde(default = "default_density")]
    pub density: String,
    /// Per-book overrides, keyed by the book's absolute path.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub books: std::collections::BTreeMap<String, BookAnnotationSettings>,
}

impl Default for AnnotationSettings {
    fn default() -> Self {
        Self {
            reader_profile: String::new(),
            level: default_level(),
            anchors: Vec::new(),
            voice: default_voice(),
            note_style: None,
            presets: Vec::new(),
            density: default_density(),
            books: std::collections::BTreeMap::new(),
        }
    }
}

fn default_density() -> String {
    "medium".into()
}
fn default_voice() -> String {
    "study".into()
}
fn default_level() -> String {
    "general".into()
}
fn default_true() -> bool {
    true
}

/// One book's remembered setup: which services were on (translate / annotate)
/// and the annotation answers given for THIS book. Prefilled from the global
/// defaults the first time a book is opened; every later edit lands here only
/// (unless 「存成我的預設」 is ticked, which also rewrites the globals).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookAnnotationSettings {
    #[serde(default = "default_true")]
    pub translate: bool,
    #[serde(default)]
    pub annotate: bool,
    #[serde(default)]
    pub reader_profile: String,
    /// Explanation level for this book: "beginner" | "general" | "insider".
    #[serde(default = "default_level")]
    pub level: String,
    /// Cognitive anchors used for this book's run.
    #[serde(default)]
    pub anchors: Vec<String>,
    /// Voice register for this book: "study" | "companion".
    #[serde(default = "default_voice")]
    pub voice: String,
    #[serde(default)]
    pub presets: Vec<String>,
    #[serde(default = "default_density")]
    pub density: String,
    /// Unix seconds of the last edit — hosts prune the oldest entries so
    /// the map can't grow without bound.
    #[serde(default)]
    pub updated_at: u64,
}

impl Default for BookAnnotationSettings {
    fn default() -> Self {
        Self {
            translate: true,
            annotate: false,
            reader_profile: String::new(),
            level: default_level(),
            anchors: Vec::new(),
            voice: default_voice(),
            presets: Vec::new(),
            density: default_density(),
            updated_at: 0,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub general: GeneralSettings,
    #[serde(default)]
    pub api: ApiSettings,
    #[serde(default)]
    pub prompts: Prompts,
    #[serde(default)]
    pub annotations: AnnotationSettings,
}

impl Settings {
    /// Load from `path`, returning defaults if the file is missing or unreadable.
    pub fn load(path: &Path) -> Settings {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        // This file contains reader profiles and absolute book paths. It is not
        // a credential store, but it is private reading data and must not inherit
        // a permissive umask on a shared Unix machine.
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            use std::time::{SystemTime, UNIX_EPOCH};

            if std::fs::symlink_metadata(path)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false)
            {
                return Err(CoreError::Other(
                    "refusing to save settings through a symlink".into(),
                ));
            }
            let dir = match path.parent() {
                Some(p) if !p.as_os_str().is_empty() => p,
                _ => Path::new("."),
            };
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("settings");
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let temp = dir.join(format!(".{name}.{}.{}.tmp", std::process::id(), nonce));
            let write_result = (|| -> std::io::Result<()> {
                let mut file = std::fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .mode(0o600)
                    .open(&temp)?;
                file.write_all(json.as_bytes())?;
                file.sync_all()?;
                std::fs::rename(&temp, path)?;
                // Best-effort directory sync makes the rename durable on
                // filesystems that support syncing directory descriptors.
                if let Ok(parent) = std::fs::File::open(dir) {
                    let _ = parent.sync_all();
                }
                Ok(())
            })();
            if write_result.is_err() {
                let _ = std::fs::remove_file(&temp);
            }
            write_result?;
        }
        #[cfg(not(unix))]
        std::fs::write(path, json)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The B3 root cause (2026-07-07 E2E): a hand-edited / pre-seeded
    // settings.json with a PARTIAL object (e.g. api without base_url siblings,
    // or general with one key) used to fail the whole parse and reset
    // EVERYTHING to factory defaults — the externally written provider/model
    // never reached the UI. Per-field defaults make partial files degrade
    // gracefully.
    #[test]
    fn partial_settings_json_keeps_the_written_fields() {
        // Only api.provider given: the rest of api defaults, general defaults.
        let s: Settings = serde_json::from_str(r#"{"api":{"provider":"anthropic"}}"#).unwrap();
        assert_eq!(s.api.provider, "anthropic");
        assert_eq!(s.api.model, "mock");
        assert_eq!(s.general.output, "bilingual");

        // provider + model pre-seeded (the E2E scenario) round-trips intact.
        let s: Settings = serde_json::from_str(
            r#"{"api":{"provider":"openai","model":"gpt-4o"},"general":{"default_mode":"expert"}}"#,
        )
        .unwrap();
        assert_eq!(s.api.provider, "openai");
        assert_eq!(s.api.model, "gpt-4o");
        assert_eq!(s.general.default_mode, "expert");
        assert_eq!(s.general.default_target_lang, "繁體中文");

        // Unknown extra keys are ignored, not fatal.
        let s: Settings =
            serde_json::from_str(r#"{"api":{"provider":"ollama","future_key":1}}"#).unwrap();
        assert_eq!(s.api.provider, "ollama");
    }

    // The saved note style + model_provider survive a save/load round-trip,
    // and old files without them still load.
    #[test]
    fn note_style_and_model_provider_roundtrip() {
        let mut s = Settings::default();
        s.annotations.note_style = Some("偏短、口語".into());
        s.api.model_provider = Some("subscription".into());
        let j = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&j).unwrap();
        assert_eq!(back.annotations.note_style.as_deref(), Some("偏短、口語"));
        assert_eq!(back.api.model_provider.as_deref(), Some("subscription"));

        let old: Settings =
            serde_json::from_str(r#"{"annotations":{"reader_profile":"工程師"}}"#).unwrap();
        assert_eq!(old.annotations.note_style, None);
        assert_eq!(old.api.model_provider, None);
    }

    // Per-book onboarding (2026-07-19): global preset/density defaults and the
    // per-book map round-trip; old files without them still load with sane
    // defaults (density "medium", empty books map).
    #[test]
    fn per_book_annotation_settings_roundtrip_and_backcompat() {
        let mut s = Settings::default();
        assert_eq!(s.annotations.density, "medium");
        s.annotations.presets = vec!["terms".into(), "history".into()];
        s.annotations.books.insert(
            "/books/wealth.epub".into(),
            BookAnnotationSettings {
                translate: false,
                annotate: true,
                reader_profile: "工程師".into(),
                presets: vec!["concepts".into()],
                density: "rich".into(),
                updated_at: 1_752_900_000,
                ..BookAnnotationSettings::default()
            },
        );
        let j = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&j).unwrap();
        assert_eq!(back.annotations.presets, vec!["terms", "history"]);
        let b = &back.annotations.books["/books/wealth.epub"];
        assert!(!b.translate && b.annotate);
        assert_eq!(b.presets, vec!["concepts"]);
        assert_eq!(b.density, "rich");
        assert_eq!(b.updated_at, 1_752_900_000);

        // Old settings.json (pre-presets) loads with defaults.
        let old: Settings =
            serde_json::from_str(r#"{"annotations":{"reader_profile":"工程師"}}"#).unwrap();
        assert!(old.annotations.presets.is_empty());
        assert_eq!(old.annotations.density, "medium");
        assert!(old.annotations.books.is_empty());
        // Partial per-book entry degrades per-field (translate defaults true).
        let part: Settings =
            serde_json::from_str(r#"{"annotations":{"books":{"/b.epub":{"annotate":true}}}}"#)
                .unwrap();
        let b = &part.annotations.books["/b.epub"];
        assert!(b.translate && b.annotate);
        assert_eq!(b.density, "medium");
    }

    #[cfg(unix)]
    #[test]
    fn saved_settings_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("translatus-settings-{nonce}"));
        let path = dir.join("settings.json");
        Settings::default().save(&path).expect("save settings");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .expect("make old settings permissive");
        Settings::default()
            .save(&path)
            .expect("repair old settings");
        let mode = std::fs::metadata(&path)
            .expect("settings metadata")
            .permissions()
            .mode()
            & 0o777;
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(mode, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn settings_save_refuses_symlink_without_touching_victim() {
        use std::os::unix::fs::symlink;
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("translatus-settings-link-{nonce}"));
        std::fs::create_dir_all(&dir).expect("test dir");
        let victim = dir.join("victim.txt");
        let path = dir.join("settings.json");
        std::fs::write(&victim, "sentinel").expect("victim");
        symlink(&victim, &path).expect("symlink");
        assert!(Settings::default().save(&path).is_err());
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "sentinel");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
