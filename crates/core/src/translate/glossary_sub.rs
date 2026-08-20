//! Deterministic glossary substitution. Before translation, locked source terms
//! are replaced with a protected `⟦Gn⟧` sentinel; the sentinel→locked-target map
//! is folded into the segment's placeholder map, so the existing write-back
//! `placeholder::restore` turns each sentinel into the locked translation. This
//! upgrades the expert glossary from a soft prompt constraint to a hard guarantee
//! that also rides the existing alignment validation for free.
//!
//! `⟦Gn⟧` is a single (atomic) token in the same family as `⟦Cn⟧`; the `G`
//! prefix can never collide with the DOM's numeric / `C`-prefixed tokens.

use crate::format::placeholder::{CLOSE, OPEN};
use crate::memory::GlossaryEntry;
use std::collections::BTreeMap;

pub struct SubResult {
    /// Source with matched terms replaced by `⟦Gn⟧` (existing `⟦n⟧`/`⟦Cn⟧` kept).
    pub substituted: String,
    /// New sentinel token (without `⟦⟧`) → locked target, to fold into placeholders.
    pub sentinels: BTreeMap<String, String>,
}

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Longest source term first (so "New York City" wins over "York"), empties dropped.
fn order_longest_first<'a>(entries: &[&'a GlossaryEntry]) -> Vec<&'a GlossaryEntry> {
    let mut v: Vec<&GlossaryEntry> = entries
        .iter()
        .copied()
        .filter(|g| !g.source.trim().is_empty() && !g.target.trim().is_empty())
        .collect();
    v.sort_by_key(|g| std::cmp::Reverse(g.source.chars().count()));
    v
}

/// Clean a locked translation before it becomes a placeholder value.
///
/// Two things are removed:
///
/// * `⟦`/`⟧`, which would corrupt the token grammar;
/// * `<` and `>`, because this value is **model-authored**. Expert mode folds
///   glossary targets into the same placeholder map the DOM uses, and
///   `placeholder::restore` inserts map values raw — it has to, since the DOM
///   entries *are* markup. Today nothing reaches the writer unescaped
///   (`bake_glossary_sentinels` folds the value into the segment target first,
///   where `escape_min` catches it), but that is an ordering guarantee living
///   in another module, and SECURITY.md promises model output can never become
///   live markup. Stripping here makes that true locally instead.
///
/// Escaping rather than stripping is wrong here: the same map feeds the plain
/// text writer, where `&lt;` would be shown to the reader verbatim. A locked
/// glossary translation has no legitimate use for angle brackets.
fn sanitize_target(t: &str) -> String {
    t.chars()
        .filter(|&c| c != OPEN && c != CLOSE && c != '<' && c != '>')
        .collect()
}

fn boundary_ok(chars: &[char], i: usize, term: &[char]) -> bool {
    let j = i + term.len();
    let left_ok = !term[0].is_ascii_alphanumeric() || i == 0 || !is_word(chars[i - 1]);
    let last = *term.last().unwrap();
    let right_ok = !last.is_ascii_alphanumeric() || j == chars.len() || !is_word(chars[j]);
    left_ok && right_ok
}

fn matches_at(chars: &[char], i: usize, term: &[char]) -> bool {
    i + term.len() <= chars.len()
        && chars[i..i + term.len()] == *term
        && boundary_ok(chars, i, term)
}

/// Single-pass substitution: protects existing `⟦…⟧` tokens, applies the longest
/// matching term at each position, and emits fresh `⟦Gn⟧` sentinels (which are
/// never re-scanned, so an inserted sentinel can't be matched by a shorter term).
pub fn substitute(source: &str, matched: &[&GlossaryEntry]) -> SubResult {
    let ordered = order_longest_first(matched);
    let terms: Vec<(Vec<char>, String)> = ordered
        .iter()
        .map(|g| (g.source.chars().collect(), sanitize_target(&g.target)))
        .collect();

    let chars: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    let mut sentinels = BTreeMap::new();
    let mut counter = 0usize;
    let mut i = 0usize;

    while i < chars.len() {
        // Copy an existing token verbatim.
        if chars[i] == OPEN {
            out.push(OPEN);
            i += 1;
            while i < chars.len() && chars[i] != CLOSE {
                out.push(chars[i]);
                i += 1;
            }
            if i < chars.len() {
                out.push(CLOSE);
                i += 1;
            }
            continue;
        }

        // Longest matching term at this position.
        let hit = terms.iter().find(|(t, _)| matches_at(&chars, i, t));
        if let Some((t, target)) = hit {
            let id = format!("G{counter}");
            counter += 1;
            sentinels.insert(id.clone(), target.clone());
            out.push(OPEN);
            out.push_str(&id);
            out.push(CLOSE);
            i += t.len();
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }

    SubResult {
        substituted: out,
        sentinels,
    }
}

#[cfg(test)]
mod tests {

    /// A glossary target is written by the model, and the map it lands in is
    /// the same one whose DOM entries are inserted as raw markup. If a hostile
    /// book can steer the pre-scan into emitting a tag as a locked
    /// translation, that tag must not be able to survive as markup anywhere.
    #[test]
    fn a_model_authored_glossary_target_cannot_carry_markup() {
        for hostile in [
            "<script>alert(1)</script>",
            "<img src=x onerror=alert(1)>",
            "safe <b>bold</b> text",
        ] {
            let out = super::sanitize_target(hostile);
            assert!(!out.contains('<'), "angle bracket survived: {out:?}");
            assert!(!out.contains('>'), "angle bracket survived: {out:?}");
        }
        // Ordinary translations are untouched, including CJK and punctuation.
        assert_eq!(super::sanitize_target("蒸汽機"), "蒸汽機");
        assert_eq!(super::sanitize_target("Smith & Co."), "Smith & Co.");
        // The token delimiters are still stripped.
        assert_eq!(super::sanitize_target("a⟦1⟧b"), "a1b");
    }
    use super::*;

    fn e(s: &str, t: &str) -> GlossaryEntry {
        GlossaryEntry {
            source: s.into(),
            target: t.into(),
            kind: "term".into(),
            first_chapter: 0,
        }
    }

    fn sub(src: &str, entries: &[GlossaryEntry]) -> SubResult {
        let refs: Vec<&GlossaryEntry> = entries.iter().collect();
        substitute(src, &refs)
    }

    #[test]
    fn basic_latin() {
        let r = sub("Captain Ahab spoke.", &[e("Ahab", "亞哈")]);
        assert_eq!(r.substituted, "Captain ⟦G0⟧ spoke.");
        assert_eq!(r.sentinels.get("G0").unwrap(), "亞哈");
    }

    #[test]
    fn word_boundary() {
        let r = sub("Ahaberration", &[e("Ahab", "亞哈")]);
        assert_eq!(r.substituted, "Ahaberration"); // right neighbour is a letter
        assert!(r.sentinels.is_empty());
    }

    #[test]
    fn cjk_no_boundary() {
        let r = sub("研究機器學習方法", &[e("機器學習", "machine learning")]);
        assert_eq!(r.substituted, "研究⟦G0⟧方法");
    }

    #[test]
    fn longest_first() {
        let r = sub(
            "機器學習與學習",
            &[e("學習", "learning"), e("機器學習", "ML")],
        );
        assert_eq!(r.substituted, "⟦G0⟧與⟦G1⟧");
        assert_eq!(r.sentinels.get("G0").unwrap(), "ML");
        assert_eq!(r.sentinels.get("G1").unwrap(), "learning");
    }

    #[test]
    fn skips_existing_tokens() {
        // term split across an inline tag → not matched
        let r = sub("前⟦1⟧Ahab⟦/1⟧後", &[e("Ahab", "亞哈")]);
        // "Ahab" sits inside the token-protected region between ⟦1⟧ and ⟦/1⟧? No —
        // it's plaintext between tokens, but boundary check: left is ⟧ (non-word),
        // right is ⟦ (non-word) → it DOES match. Ensure the surrounding tokens survive.
        assert!(r.substituted.contains("⟦1⟧"));
        assert!(r.substituted.contains("⟦/1⟧"));
        assert!(r.substituted.contains("⟦G0⟧"));
    }

    #[test]
    fn multi_occurrence() {
        let r = sub("Ahab and Ahab", &[e("Ahab", "亞哈")]);
        assert_eq!(r.substituted, "⟦G0⟧ and ⟦G1⟧");
        assert_eq!(r.sentinels.len(), 2);
        assert_eq!(r.sentinels.get("G0").unwrap(), "亞哈");
        assert_eq!(r.sentinels.get("G1").unwrap(), "亞哈");
    }

    #[test]
    fn target_with_bracket_sanitized() {
        let r = sub("Ahab", &[e("Ahab", "亞⟦哈⟧")]);
        assert_eq!(r.sentinels.get("G0").unwrap(), "亞哈");
    }
}
