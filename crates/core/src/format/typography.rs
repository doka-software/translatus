//! Target-language typography the model is not reliably consistent about.
//!
//! This is deliberately a very short list. The engine's job is to translate,
//! not to edit, so the only things normalised here are ones where the model
//! picked a character from the wrong writing system for the target — a
//! mechanical error, not a stylistic choice.
//!
//! Two rules live here:
//!
//! 1. The name interpunct (Traditional Chinese only) — see [`NAME_SEPARATORS`].
//! 2. ASCII punctuation left standing inside CJK prose — see [`PunctScript`].
//!    Models do not apply this per book, they apply it per REQUEST: in one
//!    measured annotation run (three reader profiles, same book, same model,
//!    same hour) two profiles produced clean fullwidth Chinese throughout while
//!    the third wrote "，" as "," in 10 of its 12 notes. Nothing in the prompt
//!    differed on typography, so a prompt sentence alone cannot be the fix.
//!
//! Both rules are applied by the WRITERS (`format::epub`, `format::txt`), not
//! at generation time, so they never enter `cache_signature` /
//! `annotation_signature`: an existing cache re-renders through the fix at zero
//! token cost, and nobody is re-billed.

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

/// The punctuation systems this module knows how to repair.
///
/// Korean is deliberately absent. Modern horizontal Korean sets the comma and
/// the full stop as the Latin (halfwidth) `,` and `.` followed by a space; the
/// ideographic 、 and 。 belong to older or vertical setting only. Fullwidth-ing
/// Korean notes would be an error, not a fix.
///
/// The ASCII full stop is deliberately absent from every map. Between two Han
/// characters it is ambiguous: it is as likely to be a misused NAME separator
/// (the rule above already rewrites the fullwidth ．to ‧ for exactly that
/// reason, and models that type "," for "，" also type "." for "‧") as a
/// sentence end, and guessing wrong turns 大衛.佩雷爾 into 大衛。佩雷爾.
/// Sentence ends are left to the prompt layer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PunctScript {
    /// 繁體/简体中文: ，：；！？（）. Both scripts use the same marks.
    Chinese,
    /// 日本語: the comma is 、, everything else matches Chinese.
    Japanese,
}

impl PunctScript {
    /// The fullwidth form this script wants, or `None` for a character this
    /// module does not touch.
    fn fullwidth(self, c: char) -> Option<char> {
        Some(match (self, c) {
            (PunctScript::Chinese, ',') => '，',
            (PunctScript::Japanese, ',') => '、',
            (_, ':') => '：',
            (_, ';') => '；',
            (_, '!') => '！',
            (_, '?') => '？',
            _ => return None,
        })
    }
}

/// Which punctuation system `lang` (a BCP-47 tag as produced by the writers)
/// asks for. Everything else, Korean and Latin included, is left alone.
fn punct_script(lang: &str) -> Option<PunctScript> {
    let lang = lang.trim();
    if lang == "zh" || lang.starts_with("zh-") || lang.starts_with("zh_") {
        Some(PunctScript::Chinese)
    } else if lang == "ja" || lang.starts_with("ja-") || lang.starts_with("ja_") {
        Some(PunctScript::Japanese)
    } else {
        None
    }
}

/// A character that proves we are standing inside CJK prose: Han, kana, CJK
/// punctuation, or a fullwidth form.
///
/// What is NOT here is the whole safety argument: Latin letters, digits and
/// ASCII punctuation never satisfy it, so an English sentence quoted inside a
/// note ("Are you right there, matey?"), a thousands separator (1,234), a
/// decimal (3.14), a URL, a file name and a code fragment can never have their
/// punctuation rewritten. The mark has to be surrounded by CJK on BOTH sides.
fn is_cjk_context(c: char) -> bool {
    matches!(c as u32,
        0x3000..=0x303F      // CJK symbols and punctuation: 、。〈〉《》「」『』
        | 0x3040..=0x30FF    // hiragana + katakana (・ and ー included)
        | 0x31F0..=0x31FF    // katakana phonetic extensions
        | 0x3400..=0x4DBF    // CJK extension A
        | 0x4E00..=0x9FFF    // CJK unified ideographs
        | 0xF900..=0xFAFF    // CJK compatibility ideographs
        | 0xFF01..=0xFF60    // fullwidth forms: ，：；！？（）
        | 0x20000..=0x2FA1F  // CJK extensions B and later
    )
}

fn is_kana(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30FF | 0x31F0..=0x31FF)
}

/// What happens to one character of the input.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Edit {
    Keep,
    Sub(char),
    /// The single ASCII space that followed a mark we just widened. 「成品， 是」
    /// is wrong in a way 「成品，是」 is not, and the space is only ever dropped
    /// when CJK stands on both sides of it.
    Drop,
}

/// Normalise a translated (or annotated) string for `lang`.
///
/// Traditional Chinese gets the name interpunct; Chinese and Japanese get
/// ASCII punctuation repaired inside CJK runs. Everything else passes through
/// byte for byte.
pub fn normalize(text: &str, lang: &str) -> String {
    let script = punct_script(lang);
    let interpunct = lang.starts_with("zh-Hant");
    if script.is_none() && !interpunct {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    // Inline markup lands between the halves of a name more often than one
    // would guess: publishers hang endnote anchors off the exact word they
    // annotate, so "大衛．<span id="…"/>佩雷爾" is a normal shape. Looking only
    // at the adjacent character sees `<` there and declines. Mark the tags and
    // look past them instead. The same mask keeps every rule out of attribute
    // values, where a URL's own commas and colons live.
    let in_tag = tag_mask(&chars);
    let visible_before =
        |i: usize| -> Option<char> { (0..i).rev().find(|&j| !in_tag[j]).map(|j| chars[j]) };
    let visible_after =
        |i: usize| -> Option<char> { (i + 1..chars.len()).find(|&j| !in_tag[j]).map(|j| chars[j]) };
    let visible_after_index =
        |i: usize| -> Option<usize> { (i + 1..chars.len()).find(|&j| !in_tag[j]) };

    let mut edits = vec![Edit::Keep; chars.len()];
    for (i, &c) in chars.iter().enumerate() {
        if in_tag[i] {
            continue;
        }
        let before = visible_before(i);
        let after = visible_after(i);
        if interpunct && NAME_SEPARATORS.contains(&c) {
            // At least one side must be Han: that is what makes this Chinese
            // text rather than, say, a Latin abbreviation or a katakana title,
            // where the same dot is doing a different job.
            let anchored = before.is_some_and(is_han) || after.is_some_and(is_han);
            if anchored && before.is_some_and(is_name_part) && after.is_some_and(is_name_part) {
                edits[i] = Edit::Sub(ZH_HANT_SEPARATOR);
                continue;
            }
        }
        let Some(script) = script else { continue };
        let Some(wide) = script.fullwidth(c) else {
            continue;
        };
        if !before.is_some_and(is_cjk_context) {
            continue;
        }
        // The trailing side may also be the end of the run: a note that ends
        // "他真的這樣說?" is as wrong as one that says "說?然後".
        match after {
            None => edits[i] = Edit::Sub(wide),
            Some(a) if is_cjk_context(a) || a == '\n' => edits[i] = Edit::Sub(wide),
            Some(' ') => {
                // "成品, 是工人" — the space is the model's ASCII habit too.
                let space = visible_after_index(i).expect("Some(' ') came from an index");
                if visible_after(space).is_some_and(is_cjk_context) {
                    edits[i] = Edit::Sub(wide);
                    edits[space] = Edit::Drop;
                }
            }
            Some(_) => {}
        }
    }
    if script.is_some() {
        mark_paren_pairs(&chars, &in_tag, &mut edits);
    }

    let mut out = String::with_capacity(text.len());
    for (i, &c) in chars.iter().enumerate() {
        match edits[i] {
            Edit::Keep => out.push(c),
            Edit::Sub(r) => out.push(r),
            Edit::Drop => {}
        }
    }
    out
}

/// Widen `(` and `)` only as a PAIR, and only around CJK content.
///
/// Per-character judgement is not safe here: in 「英國(及紐西蘭殖民地1921)裡」
/// the opener sits between Han and the closer does not, so an independent test
/// would emit 「英國（…1921)裡」 and leave the reader with one bracket of each
/// width. A pair is either both fullwidth or both untouched.
fn mark_paren_pairs(chars: &[char], in_tag: &[bool], edits: &mut [Edit]) {
    let mut open: Vec<usize> = Vec::new();
    for (i, &c) in chars.iter().enumerate() {
        if in_tag[i] {
            continue;
        }
        match c {
            '(' => open.push(i),
            ')' => {
                if let Some(o) = open.pop() {
                    if paren_pair_is_cjk(chars, in_tag, o, i) {
                        edits[o] = Edit::Sub('（');
                        edits[i] = Edit::Sub('）');
                    }
                }
            }
            _ => {}
        }
    }
}

/// A bracketed span qualifies when its own content is CJK prose (first and last
/// visible characters CJK, at least one Han or kana inside) and neither bracket
/// is glued to a Latin word or a number. "Mansfield(曼斯菲爾德)" and
/// "英國(1921)" therefore keep their halfwidth brackets: the first is a Latin
/// word's own parenthesis, the second holds no CJK at all.
fn paren_pair_is_cjk(chars: &[char], in_tag: &[bool], open: usize, close: usize) -> bool {
    let inner = || (open + 1..close).filter(|&j| !in_tag[j]).map(|j| chars[j]);
    let (Some(first), Some(last)) = (inner().next(), inner().next_back()) else {
        return false;
    };
    if !is_cjk_context(first) || !is_cjk_context(last) {
        return false;
    }
    if !inner().any(|c| is_han(c) || is_kana(c)) {
        return false;
    }
    let outside_ok = |c: Option<char>| c.is_none_or(|c| !c.is_ascii_alphanumeric());
    let before = (0..open).rev().find(|&j| !in_tag[j]).map(|j| chars[j]);
    let after = (close + 1..chars.len())
        .find(|&j| !in_tag[j])
        .map(|j| chars[j]);
    outside_ok(before) && outside_ok(after)
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

    // ── ASCII punctuation inside CJK prose ──────────────────────────────────

    /// The defect this rule exists for, verbatim from a measured annotation
    /// run: one reader profile out of three wrote every comma as ASCII.
    #[test]
    fn a_chinese_note_written_with_ascii_commas_is_repaired() {
        let got = normalize(
            "帳篷不是租來的成品,是工人現場搭建的大帆布棚,靠木樁跟繩索撐起。",
            "zh-Hant",
        );
        assert_eq!(
            got,
            "帳篷不是租來的成品，是工人現場搭建的大帆布棚，靠木樁跟繩索撐起。"
        );
        assert!(!got.contains(','));
    }

    #[test]
    fn every_mark_the_rule_covers_is_widened_between_han() {
        assert_eq!(
            normalize("僕役分工:男僕管粗重;女僕管室內,分得很細!真的嗎?", "zh-Hant"),
            "僕役分工：男僕管粗重；女僕管室內，分得很細！真的嗎？"
        );
    }

    /// Simplified Chinese wants exactly the same marks, and the tag may be a
    /// plain "zh" or a region code the user typed themselves.
    #[test]
    fn every_chinese_tag_gets_the_same_marks() {
        for lang in ["zh-Hans", "zh", "zh-CN", "zh-TW"] {
            assert_eq!(
                normalize("成品,是工人", lang),
                "成品，是工人",
                "lang {lang}"
            );
        }
    }

    /// Notes quote the source constantly, and an English sentence keeps English
    /// punctuation even when it is sitting inside a Chinese one.
    #[test]
    fn an_english_quotation_inside_a_chinese_note_keeps_ascii_punctuation() {
        let s = "工人喊的是「Are you right there, matey?」,不是問路。";
        assert_eq!(
            normalize(s, "zh-Hant"),
            "工人喊的是「Are you right there, matey?」，不是問路。"
        );
    }

    /// Numbers, URLs and code fragments survive because a mark is only widened
    /// with CJK on BOTH sides.
    #[test]
    fn numbers_urls_and_code_are_never_touched() {
        for s in [
            "全國共 1,234 棟與 3.14 坪",
            "參考 https://zh.example.com/a?b=1,2 的說明",
            "見 https://example.com/中文?a=1 這一頁",
            "他寫了 print(x); 這一行",
            "版本 v1.2.7 (build 9) 已發佈",
        ] {
            let got = normalize(s, "zh-Hant");
            let widened = got
                .chars()
                .filter(|c| "，：；！？（）".contains(*c))
                .count();
            assert_eq!(widened, 0, "must not widen anything in {s}, got {got}");
        }
        assert_eq!(
            normalize("全國共 1,234 棟,一棟 3.14 坪", "zh-Hant"),
            "全國共 1,234 棟，一棟 3.14 坪"
        );
    }

    /// The ASCII full stop is deliberately out of scope: between Han characters
    /// it is as likely to be a misused name separator as a sentence end.
    #[test]
    fn a_full_stop_between_han_is_left_alone() {
        for s in ["大衛.佩雷爾寫道", "他說了.然後走了", "三.五成"] {
            assert_eq!(normalize(s, "zh-Hant"), s, "must not touch {s}");
            assert_eq!(normalize(s, "ja"), s, "must not touch {s}");
        }
    }

    /// Japanese takes 、 for the comma, not ，.
    #[test]
    fn japanese_gets_its_own_comma() {
        assert_eq!(
            normalize("彼はそう言った,しかし誰も聞いていなかった", "ja"),
            "彼はそう言った、しかし誰も聞いていなかった"
        );
        assert_eq!(normalize("本当ですか?", "ja"), "本当ですか？");
        // Kana counts as CJK context on both sides.
        assert_eq!(normalize("そう,ですね", "ja"), "そう、ですね");
    }

    /// Modern horizontal Korean sets the comma and the full stop as the Latin
    /// characters followed by a space. Widening them would be the error.
    #[test]
    fn korean_punctuation_is_never_widened() {
        for s in [
            "그는 말했다, 그리고 떠났다.",
            "다음과 같다: 첫째, 둘째",
            "정말입니까?",
        ] {
            for lang in ["ko", "ko-KR"] {
                assert_eq!(normalize(s, lang), s, "Korean must pass through: {s}");
            }
        }
    }

    /// Brackets move as a pair or not at all, so a span can never end up with
    /// one fullwidth and one halfwidth bracket.
    #[test]
    fn parentheses_convert_only_as_a_matched_cjk_pair() {
        assert_eq!(
            normalize("愛德華時代的英國(及紐西蘭殖民地)裡", "zh-Hant"),
            "愛德華時代的英國（及紐西蘭殖民地）裡"
        );
        for s in [
            // Latin word's own parenthesis.
            "Mansfield(曼斯菲爾德)寫的",
            // No CJK inside.
            "英國(1921)裡",
            // Unmatched: nothing to pair with.
            "英國(及紐西蘭殖民地裡",
            "英國及紐西蘭殖民地)裡",
            // Mixed content whose closing side is a number.
            "英國(及紐西蘭殖民地1921)裡",
        ] {
            let got = normalize(s, "zh-Hant");
            assert!(
                !got.contains('（') && !got.contains('）'),
                "must not widen brackets in {s}, got {got}"
            );
        }
    }

    /// The ASCII habit brings an ASCII space with it; a fullwidth comma already
    /// carries its own spacing.
    #[test]
    fn an_ascii_space_after_a_widened_mark_is_dropped() {
        assert_eq!(normalize("成品, 是工人", "zh-Hant"), "成品，是工人");
        // Only when CJK follows: an English clause keeps its space (and its
        // comma stays halfwidth, so nothing is dropped either).
        assert_eq!(normalize("成品, and then", "zh-Hant"), "成品, and then");
    }

    /// A run may simply end after the mark.
    #[test]
    fn a_mark_at_the_end_of_the_run_is_still_widened() {
        assert_eq!(normalize("他真的這樣說?", "zh-Hant"), "他真的這樣說？");
        assert_eq!(normalize("他說,\n下一行", "zh-Hant"), "他說，\n下一行");
        // ...but a mark with nothing CJK in front of it is not ours.
        assert_eq!(normalize("matey?", "zh-Hant"), "matey?");
    }

    /// Markup is structure, not prose: a URL's own punctuation lives inside
    /// attributes and must survive untouched.
    #[test]
    fn punctuation_inside_markup_is_untouched() {
        let s = r#"<a href="https://example.com/a?b=1,2">中文,中文</a>"#;
        let got = normalize(s, "zh-Hant");
        assert!(
            got.contains(r#"href="https://example.com/a?b=1,2""#),
            "attribute must survive: {got}"
        );
        assert!(got.contains(">中文，中文<"), "got {got}");
    }

    /// English, and anything else this module does not claim to know, still
    /// passes through byte for byte.
    #[test]
    fn non_cjk_targets_keep_every_ascii_mark() {
        let s = "He said, \"yes\"; then left (quickly). Really?";
        for lang in ["en", "ko", "fr", "de-DE"] {
            assert_eq!(normalize(s, lang), s, "lang {lang}");
        }
    }
}
