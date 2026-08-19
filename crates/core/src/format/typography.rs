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

/// Normalise a translated string for `lang` (a BCP-47 tag as produced by the
/// writers). Only Traditional Chinese is touched today.
pub fn normalize(text: &str, lang: &str) -> String {
    if !lang.starts_with("zh-Hant") {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    for (i, &c) in chars.iter().enumerate() {
        if NAME_SEPARATORS.contains(&c) {
            let before = i.checked_sub(1).and_then(|j| chars.get(j)).copied();
            let after = chars.get(i + 1).copied();
            if before.is_some_and(is_han) && after.is_some_and(is_han) {
                out.push(ZH_HANT_SEPARATOR);
                continue;
            }
        }
        out.push(c);
    }
    out
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

    #[test]
    fn other_target_languages_are_never_rewritten() {
        let s = "Winston・Churchill";
        for lang in ["ja", "en", "zh-Hans", "ko"] {
            assert_eq!(normalize(s, lang), s);
        }
    }
}
