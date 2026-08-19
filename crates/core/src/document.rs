//! In-memory intermediate representation produced by parsing and consumed by
//! translation + reassembly. Format-agnostic: EPUB, TXT and (later) DOCX all
//! normalise into `Book` → `Chapter` → `Segment`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Source file format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Epub,
    Txt,
}

impl Format {
    pub fn detect(path: &std::path::Path) -> Option<Format> {
        match path
            .extension()
            .and_then(|e| e.to_str())?
            .to_ascii_lowercase()
            .as_str()
        {
            "epub" => Some(Format::Epub),
            "txt" => Some(Format::Txt),
            _ => None,
        }
    }
}

/// Where an annotation is anchored relative to the block it annotates (AN-014).
/// `Before` = background the reader needs BEFORE the paragraph (鋪墊);
/// `After` = extension/explanation best read AFTER it. Decided by the
/// selection pass; the writing and review passes may never change it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum NotePos {
    Before,
    #[default]
    After,
}

/// One reader-personalised annotation (眉批): placement + text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Note {
    #[serde(default)]
    pub pos: NotePos,
    pub text: String,
}

impl Note {
    pub fn new(pos: NotePos, text: impl Into<String>) -> Self {
        Note {
            pos,
            text: text.into(),
        }
    }

    /// A note anchored after its block — the historical default placement.
    pub fn after(text: impl Into<String>) -> Self {
        Note::new(NotePos::After, text)
    }

    /// The deliberate "no note here" decision (cached so resume never re-asks).
    pub fn skip() -> Self {
        Note::after(String::new())
    }

    /// True when this is the deliberate "not annotated" decision.
    pub fn is_skip(&self) -> bool {
        self.text.trim().is_empty()
    }
}

/// Back-compat deserializer: notes serialized before AN-014 were plain strings
/// (placement was always "after the block"), so a bare string still loads.
impl<'de> Deserialize<'de> for Note {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Compat {
            Full {
                #[serde(default)]
                pos: NotePos,
                text: String,
            },
            Text(String),
        }
        Ok(match Compat::deserialize(deserializer)? {
            Compat::Full { pos, text } => Note { pos, text },
            Compat::Text(text) => Note::after(text),
        })
    }
}

/// The smallest translatable unit: one block's text content, with inline tags
/// already replaced by `⟦n⟧…⟦/n⟧` / `⟦Cn⟧` placeholders.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    /// Sequential index of the source block within its chapter (the write-back key).
    pub block_index: usize,
    /// Source text with placeholders substituted for inline markup.
    pub source: String,
    /// placeholder token -> original inline markup (opening, closing or self-contained).
    pub placeholders: BTreeMap<String, String>,
    /// Filled once translated.
    pub target: Option<String>,
    /// Reader-personalised annotation (眉批). `None` = not yet decided;
    /// `Some(skip)` (empty text) = deliberately not annotated (cached so resume
    /// never re-asks); `Some(note)` = a note rendered before/after this block
    /// per `note.pos`. `serde(default)` keeps cache JSON written before this
    /// field existed loading fine.
    #[serde(default)]
    pub note: Option<Note>,
}

impl Segment {
    pub fn new(block_index: usize, source: String, placeholders: BTreeMap<String, String>) -> Self {
        Self {
            block_index,
            source,
            placeholders,
            target: None,
            note: None,
        }
    }

    /// Rough token estimate (≈ chars/4 for latin, but CJK is ~1 token/char; we use a blended 2.5).
    /// Rough token count for this segment's source, as it will be sent.
    ///
    /// Scripts do not tokenise alike: Latin text runs about 2.5 characters to
    /// the token, while Han, kana and Hangul run closer to one character each
    /// (often worse, once a tokenizer has to fall back to bytes). Counting a
    /// Japanese book at the Latin rate under-reports it by roughly 2.5x before
    /// anything else goes wrong, which is the wrong direction for a number the
    /// user is asked to agree to.
    pub fn est_tokens(&self) -> usize {
        let mut dense = 0usize; // one token or worse per character
        let mut light = 0usize; // ~2.5 characters per token
        for c in self.source.chars() {
            if is_dense_script(c) {
                dense += 1;
            } else {
                light += 1;
            }
        }
        dense + ((light as f32) / 2.5).ceil() as usize
    }
}

/// CJK and Hangul, where a character is worth about a token.
fn is_dense_script(c: char) -> bool {
    matches!(
        c as u32,
        0x3040..=0x30FF      // hiragana, katakana
            | 0x3400..=0x4DBF // CJK ext A
            | 0x4E00..=0x9FFF // CJK unified
            | 0xF900..=0xFAFF // compatibility ideographs
            | 0xAC00..=0xD7AF // Hangul syllables
            | 0xFF66..=0xFF9F // halfwidth katakana
    )
}

/// One spine document (EPUB) or the whole file (TXT).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chapter {
    /// Linear reading order.
    pub spine_index: usize,
    /// EPUB: the zip path of the XHTML. TXT: a synthetic name.
    pub href: String,
    pub title: Option<String>,
    pub segments: Vec<Segment>,
}

impl Chapter {
    pub fn total_segments(&self) -> usize {
        self.segments.len()
    }
}

/// A parsed book ready for translation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Book {
    pub format: Format,
    pub chapters: Vec<Chapter>,
    /// Source title from metadata, if any (not translated by default).
    pub title: Option<String>,
}

impl Book {
    pub fn total_segments(&self) -> usize {
        self.chapters.iter().map(|c| c.total_segments()).sum()
    }

    pub fn est_source_tokens(&self) -> usize {
        self.chapters
            .iter()
            .flat_map(|c| c.segments.iter())
            .map(|s| s.est_tokens())
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Japanese sentence must not be priced like an English one. Counting
    /// every script at 2.5 characters per token under-reported a CJK book by
    /// about 2.5x before anything else went wrong.
    #[test]
    fn cjk_costs_about_a_token_a_character_and_latin_does_not() {
        let ja = Segment::new(0, "定期テストも終わった五月下旬".into(), BTreeMap::new());
        let en = Segment::new(
            0,
            "The ferry left before the light did.".into(),
            BTreeMap::new(),
        );

        assert_eq!(ja.est_tokens(), ja.source.chars().count());
        assert!(
            en.est_tokens() < en.source.chars().count() / 2,
            "latin must stay well under one token per character, got {}",
            en.est_tokens()
        );

        // Mixed text splits: the Han runs dense, the ASCII light.
        let mixed = Segment::new(0, "第 1 章 Introduction".into(), BTreeMap::new());
        assert!(mixed.est_tokens() >= 1);
        assert!(mixed.est_tokens() < mixed.source.chars().count());
    }

    // Pre-AN-014 notes were plain strings: they must still deserialize, as
    // "after" placement (the only placement that existed back then).
    #[test]
    fn note_deserializes_legacy_plain_string() {
        let n: Note = serde_json::from_str(r#""歷史背景說明""#).unwrap();
        assert_eq!(n, Note::after("歷史背景說明"));

        let n: Note = serde_json::from_str(r#"{"pos":"before","text":"鋪墊"}"#).unwrap();
        assert_eq!(n, Note::new(NotePos::Before, "鋪墊"));

        // pos missing in the object form also defaults to after.
        let n: Note = serde_json::from_str(r#"{"text":"補充"}"#).unwrap();
        assert_eq!(n.pos, NotePos::After);

        // Roundtrip of the new form.
        let j = serde_json::to_string(&Note::new(NotePos::Before, "x")).unwrap();
        assert_eq!(j, r#"{"pos":"before","text":"x"}"#);
    }

    #[test]
    fn note_skip_semantics() {
        assert!(Note::skip().is_skip());
        assert!(Note::after("  ").is_skip());
        assert!(!Note::new(NotePos::Before, "內容").is_skip());
    }
}
