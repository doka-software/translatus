//! Structured memory for the expert (multi-pass) level: a locked glossary
//! (proper nouns / terms with a first-occurrence translation), a book-wide style
//! guide, and a rolling bilingual summary. This is what gives expert level its
//! book-level consistency — the difference from sentence level is the algorithm,
//! not the prompt. Serializable so it can be cached in the job store for resume.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlossaryEntry {
    pub source: String,
    /// Locked at first occurrence; applied book-wide.
    pub target: String,
    /// "person" | "place" | "org" | "term" | …
    pub kind: String,
    pub first_chapter: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExpertMemory {
    pub glossary: Vec<GlossaryEntry>,
    pub style_guide: String,
    /// Rolling bilingual summary (compressed, not the full prior text).
    pub summary: String,
}

impl ExpertMemory {
    /// Glossary entries whose source term appears in `text` (lexical match).
    /// Only these get injected into a chapter's prompt — not the whole book.
    pub fn matched<'a>(&'a self, text: &str) -> Vec<&'a GlossaryEntry> {
        self.glossary
            .iter()
            .filter(|g| !g.source.is_empty() && text.contains(&g.source))
            .collect()
    }

    /// Merge newly discovered terms, locking the first occurrence (never overwrite).
    pub fn add_terms(&mut self, terms: impl IntoIterator<Item = GlossaryEntry>) {
        for t in terms {
            if t.source.trim().is_empty() || t.target.trim().is_empty() {
                continue;
            }
            if !self.glossary.iter().any(|g| g.source == t.source) {
                self.glossary.push(t);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(s: &str, t: &str) -> GlossaryEntry {
        GlossaryEntry {
            source: s.into(),
            target: t.into(),
            kind: "term".into(),
            first_chapter: 0,
        }
    }

    #[test]
    fn matched_finds_present_terms() {
        let mut m = ExpertMemory::default();
        m.add_terms([entry("Ahab", "亞哈"), entry("Pequod", "裴廓德號")]);
        let hits = m.matched("Captain Ahab stood on deck.");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].target, "亞哈");
    }

    #[test]
    fn add_terms_locks_first() {
        let mut m = ExpertMemory::default();
        m.add_terms([entry("Ahab", "亞哈")]);
        m.add_terms([entry("Ahab", "阿哈伯")]); // ignored — already locked
        assert_eq!(m.glossary.len(), 1);
        assert_eq!(m.glossary[0].target, "亞哈");
    }
}
