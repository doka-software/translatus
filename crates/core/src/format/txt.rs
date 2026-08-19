//! Plain-text adapter. One chapter; each blank-line-separated paragraph is a
//! segment with no inline markup. Used to exercise the chunk/cache/resume spine
//! on the simplest possible format.

use crate::config::OutputMode;
use crate::document::{Book, Chapter, Format, Segment};
use crate::error::Result;
use std::path::Path;

pub struct TxtDoc {
    /// Original paragraphs (kept so untranslated ones survive verbatim).
    pub paragraphs: Vec<String>,
}

fn split_paragraphs(text: &str) -> Vec<String> {
    // Normalise CRLF, split on blank lines, keep non-empty paragraphs.
    let norm = text.replace("\r\n", "\n");
    norm.split("\n\n").map(|p| p.to_string()).collect()
}

pub fn extract(path: &Path) -> Result<(Book, TxtDoc)> {
    let text = std::fs::read_to_string(path)?;
    let paragraphs = split_paragraphs(&text);
    let mut segments = Vec::new();
    for (i, p) in paragraphs.iter().enumerate() {
        if p.trim().is_empty() {
            continue;
        }
        segments.push(Segment::new(i, p.clone(), Default::default()));
    }
    let chapter = Chapter {
        apparatus: false,
        spine_index: 0,
        href: "content.txt".to_string(),
        title: None,
        segments,
    };
    let book = Book {
        format: Format::Txt,
        chapters: vec![chapter],
        title: None,
    };
    Ok((book, TxtDoc { paragraphs }))
}

pub fn write(doc: &TxtDoc, book: &Book, out: &Path, mode: OutputMode, lang: &str) -> Result<()> {
    let lang = super::epub::lang_attr(lang);
    let mut paras = doc.paragraphs.clone();
    if let Some(chapter) = book.chapters.first() {
        for seg in &chapter.segments {
            if let Some(t) = &seg.target {
                // Restore inline / glossary placeholders (⟦n⟧, ⟦Cn⟧, ⟦Gn⟧) the same
                // way the EPUB writer does; otherwise expert-mode glossary sentinels
                // leak as literal ⟦Gn⟧ into the output text.
                let restored = crate::format::typography::normalize(
                    &crate::format::placeholder::restore(t, &seg.placeholders),
                    &lang,
                );
                if let Some(slot) = paras.get_mut(seg.block_index) {
                    *slot = match mode {
                        OutputMode::Replace => restored,
                        OutputMode::Bilingual => format!("{}\n{}", slot, restored),
                    };
                }
            }
            // Annotation: its own marked line on the side its placement
            // dictates (AN-014) — before the paragraph (背景鋪墊) or after it
            // (and after the translation in bilingual mode), same as the EPUB.
            if let Some(n) = seg.note.as_ref().filter(|n| !n.is_skip()) {
                if let Some(slot) = paras.get_mut(seg.block_index) {
                    *slot = match n.pos {
                        crate::document::NotePos::Before => {
                            format!("{} {}\n{}", crate::format::dom::NOTE_PREFIX, n.text, slot)
                        }
                        crate::document::NotePos::After => {
                            format!("{}\n{} {}", slot, crate::format::dom::NOTE_PREFIX, n.text)
                        }
                    };
                }
            }
        }
    }
    super::atomic_write(out, paras.join("\n\n").as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Format, Segment};

    /// Regression: the TXT writer must restore placeholders (incl. expert-mode
    /// ⟦Gn⟧ glossary sentinels), not emit them literally. See 2026-06-06 fix.
    #[test]
    fn write_restores_glossary_sentinels() {
        let out = std::env::temp_dir().join("et_txt_restore_test.txt");
        let mut seg = Segment::new(0, "Ahab".into(), Default::default());
        seg.placeholders.insert("G0".into(), "亞哈".into());
        seg.target = Some("⟦G0⟧船長".into());
        let chapter = Chapter {
            spine_index: 0,
            href: "c".into(),
            title: None,
            segments: vec![seg],
            apparatus: false,
        };
        let book = Book {
            format: Format::Txt,
            chapters: vec![chapter],
            title: None,
        };
        let doc = TxtDoc {
            paragraphs: vec!["Ahab".into()],
        };
        write(&doc, &book, &out, OutputMode::Replace, "English").unwrap();
        let got = std::fs::read_to_string(&out).unwrap();
        let _ = std::fs::remove_file(&out);
        assert_eq!(got.trim(), "亞哈船長");
    }
}
