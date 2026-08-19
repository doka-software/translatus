//! Placeholder protocol for inline markup.
//!
//! Inline elements that wrap text become a numbered, paired placeholder:
//!   `<b>red</b>`  →  `⟦1⟧red⟦/1⟧`
//! Atomic / non-translatable inline content (images, `<br/>`, code spans we keep
//! verbatim) becomes a single placeholder: `⟦C1⟧`.
//!
//! The LLM is told to preserve every placeholder exactly (count, id, pairing),
//! moving them only as word order requires. We re-validate after translation and
//! again after reassembly; mismatches force a retry / fallback.

pub const OPEN: char = '⟦';
pub const CLOSE: char = '⟧';

/// Extract every placeholder token (the text between ⟦ and ⟧) in order of appearance.
pub fn tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = s.char_indices().peekable();
    while let Some((_, c)) = chars.next() {
        if c == OPEN {
            let mut inner = String::new();
            for (_, c2) in chars.by_ref() {
                if c2 == CLOSE {
                    out.push(inner);
                    break;
                }
                inner.push(c2);
            }
        }
    }
    out
}

/// True if the two strings carry the exact same multiset of placeholder tokens.
pub fn same_multiset(a: &str, b: &str) -> bool {
    let mut ta = tokens(a);
    let mut tb = tokens(b);
    ta.sort();
    tb.sort();
    ta == tb
}

/// Validate that `target` preserved `source`'s placeholders. Returns a human
/// readable reason on failure.
pub fn validate(source: &str, target: &str) -> Result<(), String> {
    if same_multiset(source, target) {
        Ok(())
    } else {
        Err(format!(
            "placeholder mismatch: source has {:?}, target has {:?}",
            tokens(source),
            tokens(target)
        ))
    }
}

/// Remove every `⟦…⟧` token, leaving only the human-visible text. Used when a
/// pass needs readable prose (pre-scan sampling, annotation payloads) rather
/// than the translation protocol form.
pub fn strip_tokens(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tok = false;
    for c in s.chars() {
        match c {
            c if c == OPEN => in_tok = true,
            c if c == CLOSE => in_tok = false,
            c if !in_tok => out.push(c),
            _ => {}
        }
    }
    out
}

/// Restore the original inline markup in a translated string using the map built
/// during extraction. Unknown placeholders are left as-is (caught by validation).
pub fn restore(translated: &str, map: &std::collections::BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(translated.len());
    let mut chars = translated.char_indices().peekable();
    while let Some((_, c)) = chars.next() {
        if c == OPEN {
            let mut inner = String::new();
            let mut closed = false;
            for (_, c2) in chars.by_ref() {
                if c2 == CLOSE {
                    closed = true;
                    break;
                }
                inner.push(c2);
            }
            if closed {
                if let Some(markup) = map.get(&inner) {
                    out.push_str(markup);
                } else {
                    // leave the raw placeholder so validation can flag it
                    out.push(OPEN);
                    out.push_str(&inner);
                    out.push(CLOSE);
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn tokens_in_order() {
        assert_eq!(tokens("a ⟦1⟧b⟦/1⟧ ⟦C2⟧"), vec!["1", "/1", "C2"]);
    }

    #[test]
    fn multiset_ignores_order() {
        assert!(same_multiset("⟦1⟧x⟦/1⟧", "y⟦/1⟧z⟦1⟧"));
        assert!(!same_multiset("⟦1⟧", "⟦1⟧⟦1⟧"));
    }

    #[test]
    fn restore_roundtrip() {
        let mut m = BTreeMap::new();
        m.insert("1".to_string(), "<b>".to_string());
        m.insert("/1".to_string(), "</b>".to_string());
        assert_eq!(restore("一⟦1⟧紅⟦/1⟧車", &m), "一<b>紅</b>車");
    }
}
