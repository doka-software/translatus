//! Target-language typography the model is not reliably consistent about.
//!
//! This is deliberately a very short list. The engine's job is to translate,
//! not to edit, so the only things normalised here are ones where the model
//! picked a character from the wrong writing system for the target — a
//! mechanical error, not a stylistic choice.

/// The interpunct that separates the parts of a transliterated foreign name.
///
/// Traditional Chinese wants U+2027 (the Big5 間隔號, and what the Ministry of
/// Education's punctuation handbook prints). Models reach for U+30FB (the
/// Japanese katakana middle dot) and U+00B7 (the mainland GB middle dot)
/// instead, and mix all three inside one book: measured over one 534-segment
/// translation, U+30FB appeared 30 times, U+00B7 16, and the correct U+2027
/// only 4.
///
/// U+FF0E (fullwidth full stop) is included, but only under the same
/// between-two-Han-characters guard: a Chinese sentence ends with 。, never
/// with ．, so a fullwidth stop sitting between two Han characters is a
/// misused separator rather than punctuation. The same substitution is what
/// 《禮記‧禮運》 wants between a book and a chapter name.
const NAME_SEPARATORS: [char; 3] = ['\u{30FB}', '\u{00B7}', '\u{FF0E}'];
const ZH_HANT_SEPARATOR: char = '\u{2027}';

fn is_han(c: char) -> bool {
    matches!(c as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF)
}

/// Something that can be part of a transliterated name: a Han character, or a
/// Latin letter standing in for an initial ("約翰‧D‧洛克斐勒",
/// "F‧史考特‧費茲傑羅").
fn is_name_part(c: char) -> bool {
    is_han(c) || c.is_ascii_alphabetic()
}

/// Normalise a translated string for `lang` (a BCP-47 tag as produced by the
/// writers). Only Traditional Chinese is touched today.
pub fn normalize(text: &str, lang: &str) -> String {
    if !lang.starts_with("zh-Hant") {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    // Inline markup lands between the halves of a name more often than one
    // would guess: publishers hang endnote anchors off the exact word they
    // annotate, so "大衛．<span id="…"/>佩雷爾" is a normal shape. Looking only
    // at the adjacent character sees `<` there and declines. Mark the tags and
    // look past them instead.
    let in_tag = tag_mask(&chars);
    let visible_before =
        |i: usize| -> Option<char> { (0..i).rev().find(|&j| !in_tag[j]).map(|j| chars[j]) };
    let visible_after =
        |i: usize| -> Option<char> { (i + 1..chars.len()).find(|&j| !in_tag[j]).map(|j| chars[j]) };

    let mut out = String::with_capacity(text.len());
    for (i, &c) in chars.iter().enumerate() {
        if NAME_SEPARATORS.contains(&c) && !in_tag[i] {
            let before = visible_before(i);
            let after = visible_after(i);
            // At least one side must be Han: that is what makes this Chinese
            // text rather than, say, a Latin abbreviation or a katakana title,
            // where the same dot is doing a different job.
            let anchored = before.is_some_and(is_han) || after.is_some_and(is_han);
            if anchored && before.is_some_and(is_name_part) && after.is_some_and(is_name_part) {
                out.push(ZH_HANT_SEPARATOR);
                continue;
            }
        }
        out.push(c);
    }
    out
}

/// Which characters sit inside a `<…>` tag. Quotes are honoured so a `>` in an
/// attribute value does not close a tag early.
fn tag_mask(chars: &[char]) -> Vec<bool> {
    let mut mask = vec![false; chars.len()];
    let mut depth = false;
    let mut quote: Option<char> = None;
    for (i, &c) in chars.iter().enumerate() {
        if depth {
            mask[i] = true;
            match quote {
                Some(q) if c == q => quote = None,
                Some(_) => {}
                None if c == '"' || c == '\'' => quote = Some(c),
                None if c == '>' => depth = false,
                None => {}
            }
        } else if c == '<' {
            depth = true;
            mask[i] = true;
        }
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_book_ends_up_with_one_interpunct() {
        let mixed = "溫斯頓・邱吉爾、馬塞爾·普魯斯特、讓．保羅與亞當‧斯密";
        let got = normalize(mixed, "zh-Hant");
        assert_eq!(got, "溫斯頓‧邱吉爾、馬塞爾‧普魯斯特、讓‧保羅與亞當‧斯密");
        assert_eq!(got.matches('\u{2027}').count(), 4);
        for wrong in ['\u{30FB}', '\u{00B7}', '\u{FF0E}'] {
            assert!(!got.contains(wrong), "U+{:04X} must be gone", wrong as u32);
        }
    }

    /// Only between Han characters: a middle dot doing any other job — a
    /// bullet, a Latin abbreviation, katakana that stayed katakana — is left
    /// exactly as the model wrote it.
    #[test]
    fn a_dot_that_is_not_a_name_separator_is_untouched() {
        for s in [
            "A·B",
            "ドラゴン・クエスト",
            "・開頭",
            "結尾・",
            "3·5",
            "3．14",
        ] {
            assert_eq!(normalize(s, "zh-Hant"), s, "must not touch {s}");
        }
    }

    /// Publishers hang endnote anchors off the exact word they annotate, so
    /// markup between the halves of a name is normal, not exotic.
    #[test]
    fn markup_between_the_halves_of_a_name_does_not_hide_it() {
        let s = r#"我的朋友大衛．<span id="EndnotePhraseInText56"/>佩雷爾曾經寫道"#;
        let got = normalize(s, "zh-Hant");
        assert!(got.contains("大衛‧<span"), "got {got}");
        assert!(!got.contains('\u{FF0E}'));
        // The tag itself is untouched, attributes and all.
        assert!(got.contains(r#"<span id="EndnotePhraseInText56"/>"#));
    }

    /// A separator that is only inside an attribute is not text.
    #[test]
    fn a_dot_inside_an_attribute_is_left_alone() {
        let s = r#"<a href="a・b.html">王・李</a>"#;
        let got = normalize(s, "zh-Hant");
        assert!(
            got.contains(r#"href="a・b.html""#),
            "attribute must survive: {got}"
        );
        assert!(got.contains("王‧李"));
    }

    /// A Latin initial inside a Chinese name keeps the same separator.
    #[test]
    fn a_latin_initial_inside_a_chinese_name_is_still_a_name() {
        assert_eq!(normalize("約翰．D．洛克斐勒", "zh-Hant"), "約翰‧D‧洛克斐勒");
        assert_eq!(
            normalize("F·史考特·費茲傑羅", "zh-Hant"),
            "F‧史考特‧費茲傑羅"
        );
    }

    #[test]
    fn other_target_languages_are_never_rewritten() {
        let s = "Winston・Churchill";
        for lang in ["ja", "en", "zh-Hans", "ko"] {
            assert_eq!(normalize(s, lang), s);
        }
    }
}
