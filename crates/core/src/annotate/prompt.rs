//! System prompts for the annotation passes (眉批). Like the translation
//! prompts, they split into a **locked hard-rules section** (the non-negotiable
//! content contract) and an **editable style layer** (tone / depth / length
//! preferences the user may override — see `DEFAULT_NOTE_STYLE`):
//!
//! Hard rules (locked, shared verbatim by N1 writing and N2 review):
//! - Notes are NEUTRAL background & explanation (history, facts, term origins,
//!   the author's circumstances, text structure) — concrete and information-dense.
//! - The reader profile decides only WHERE to stop, WHAT angle and HOW deep.
//!   The note text itself never addresses the reader ("就像你…", "跟你一樣" are
//!   intrusive and banned) and never reviews the book ("全書最常被引用的段落",
//!   "值得一讀" are meta commentary and banned). The "aha" belongs to the reader.
//! - Notes speak AS margin notes; they never impersonate the book.
//! - No placeholders / HTML, no re-narration, no duplicate topics.
//!
//! Style layer (editable): length target, tone, depth preference. Sanitised
//! (length-capped) and injected INSIDE the locked rules, so it can tune quality
//! but never break the contract. Part of `annotation_signature` — editing the
//! style re-annotates, never re-bills the translation. Tuning guide:
//! docs/ANNOTATION-TUNING.md.
//!
//! Preset help angles (`AnnotationConfig::presets`): fixed ids the reader can
//! tick without typing (App chips / CLI `--note-presets`). Each id maps to one
//! guidance sentence in [`PRESETS`]; the block is injected right AFTER the
//! reader's free-text profile in both the selection pass (angle guidance) and
//! the writing pass. Unknown ids are ignored (callers warn).

use crate::config::{AnnotationConfig, Density};
use crate::translate::prompt::sanitize_style;

/// Protocol markers. They open each system prompt so the offline mock provider
/// can recognise an annotation request deterministically; real models simply
/// read them as a task tag.
pub const N0_MARKER: &str = "[[TRANSLATUS:ANNOTATE:PLAN]]";
pub const NSEL_MARKER: &str = "[[TRANSLATUS:ANNOTATE:SELECT]]";
pub const N1_MARKER: &str = "[[TRANSLATUS:ANNOTATE:NOTES]]";
pub const N2_MARKER: &str = "[[TRANSLATUS:ANNOTATE:REVIEW]]";

/// Default length target for one note (characters, target-language). Quality
/// parameter, not a hard rule: it lives in `DEFAULT_NOTE_STYLE`, which a user
/// style overrides.
pub const NOTE_MAX_CHARS: usize = 160;

/// The engine-default note STYLE paragraph — the editable quality layer
/// (length target, tone, information density). "Restore default" in the UI
/// returns this; a user style replaces it wholesale. The hard rules stay
/// locked either way.
pub const DEFAULT_NOTE_STYLE: &str = "語氣如頁邊小字：乾淨、克制、資訊密度高，寧短勿長，每則以 160 字為長度上限目標。用該目標語言母語者的自然口吻寫，不要翻譯腔、不要文謅謅的書面語、不要滲入其他地區的慣用語。以繁體中文書寫時，用字遵臺灣正體慣用（想像不作想象、裡面不作里面、資訊不作信息、透過不作通过），不滲入其他中文區的慣用語與字形。第一句就給具體的東西：年代、人名、實物、數字或一個明確斷言，不以「這段」「此處」開頭。一則只做一件事；只寫正文給不了的東西。評價詞必須釘住具體的字句或動作。讀者必然誤解之處優先補正：直接給事實，不寫「你可能以為」。長度隨內容起伏，一句話講得完就一句話收；同一本書要長短交錯，別每則都一樣長。多層資訊按有感程度排序，最有感的先出。在適當時機，可把正文接到書外真實世界的脈絡（後世發展、別的領域、歷史長河裡的同類事件），讓連結感落在整個世界而不只這本書——但這類外部連結必須是你有把握、查得到的真事實，寧可不連，絕不可為了漂亮而杜撰。斷言須可查證；寧可明說「來源不明」，也不為了收得漂亮而下沒有根據的斷語。外部事實只說到自己有把握的程度：不加「幾乎都」「一律」「完全」這類絕對化量詞，常見就寫常見；外部類比與後世連結是「延伸」，不是「結論」：不把類比寫成因果，不把「常被視為最早/源頭之一」寫成「直接源頭」；法律與制度的運作機制寧可少講一步，也不簡化到錯。引用書外文本的內容（序言、書信、別的著作）時，必須確知那段文字真的這樣說，不確知就不引。";

/// The companion-voice default STYLE paragraph (`NoteVoice::Companion`): the
/// same information discipline as the study default, in a friend-at-your-side
/// register with a deliberately higher share of short reaction notes. Selected
/// only when the user has no custom style; the hard rules (neutrality, never
/// addressing the reader, no spoilers) are identical for both voices.
pub const COMPANION_NOTE_STYLE: &str = "口吻像一個懂行的朋友坐在旁邊一起讀：口語、自然、有反應，但每一則仍要給正文給不了的東西，每則以 160 字為長度上限目標。用該目標語言母語者的日常口吻寫，不要翻譯腔、不要書面腔。以繁體中文書寫時，用字遵臺灣正體慣用（想像不作想象、裡面不作里面、資訊不作信息、透過不作通过），不滲入其他中文區的慣用語與字形。長短要有明顯落差：大約三分之一的眉批可以只是一句短反應（15 字以內）：點出一個妙處、標記一個轉折、替一個伏筆收線，評價詞必須釘住具體的字句或動作；其餘寫成事實批，第一句就給具體的東西（年代、人名、實物、數字或一個明確斷言），不以「這段」「此處」開頭。一則只做一件事。讀者必然誤解之處優先補正：直接給事實，不寫「你可能以為」。在適當時機把正文接到書外真實世界的脈絡（後世發展、別的領域、同類事件），但這類連結必須是查得到的真事實，寧可不連，絕不可為了有趣而杜撰。斷言須可查證；來源不明就明說。外部事實不加「幾乎都」「一律」「完全」這類絕對化量詞，說到有把握的程度為止；外部類比與後世連結是「延伸」，不是「結論」：不把類比寫成因果，不把「常被視為最早/源頭之一」寫成「直接源頭」；法律與制度的運作機制寧可少講一步，也不簡化到錯。引用書外文本內容時必須確知原文真的這樣說。";

/// The engine-default style paragraph for a voice. A user style overrides
/// whichever voice is set (see `style_block`).
pub fn default_style_for(voice: crate::config::NoteVoice) -> &'static str {
    match voice {
        crate::config::NoteVoice::Study => DEFAULT_NOTE_STYLE,
        crate::config::NoteVoice::Companion => COMPANION_NOTE_STYLE,
    }
}

/// Cognitive-anchor list caps: entries beyond `ANCHOR_MAX_COUNT` fall off, and
/// each entry is trimmed to `ANCHOR_MAX_CHARS` — an anchor is a short label
/// ("軟體工程師", "讀過《國富論》"), not a paragraph.
pub const ANCHOR_MAX_COUNT: usize = 16;
pub const ANCHOR_MAX_CHARS: usize = 80;

/// Canonicalise the reader's cognitive anchors: trimmed, non-empty, deduped in
/// first-seen order, each capped to `ANCHOR_MAX_CHARS` chars, at most
/// `ANCHOR_MAX_COUNT` entries. Like `canonical_presets`, this is the ONLY form
/// the prompts and the annotation signature consume.
pub fn canonical_anchors(raw: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for a in raw {
        let a: String = a.trim().chars().take(ANCHOR_MAX_CHARS).collect();
        if !a.is_empty() && !out.contains(&a) {
            out.push(a);
        }
        if out.len() >= ANCHOR_MAX_COUNT {
            break;
        }
    }
    out
}

/// The injected cognitive-anchor block (empty when the reader listed none).
/// Sits right after the preset block in the planning, selection and writing
/// prompts: the profile says why they read, the anchors say what ground the
/// notes may bridge FROM.
fn anchors_block(anno: &AnnotationConfig) -> String {
    let anchors = canonical_anchors(&anno.anchors);
    if anchors.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = anchors.iter().map(|a| format!("- {a}")).collect();
    format!(
        "\n\n# 讀者的認知錨（讀者已熟悉的領域與經驗；僅用於選點與搭橋，內文不得提及讀者）\n{}\n用法：解釋新概念時，優先從這些熟悉領域搭橋（類比、對照、同構的機制），讓讀者用既有認知理解新內容。但類比必須準確：兩者機制真的同構才用；搭不準寧可不搭，錯的類比比沒有類比更傷理解。錨只決定「怎麼講」，內文永遠不提讀者本人。",
        lines.join("\n")
    )
}

/// Generic-opener check (具體開頭律, machine-enforced): the first sentence of
/// a note must give something concrete — a note that opens with "這段…"-style
/// framing is filler by construction and is rejected at the same chokepoints
/// as a reader-addressing note. Conservative prefix list: a quoted opener
/// (「這段話」 as a quotation) starts with a quote mark and passes.
pub fn note_opens_generic(note: &str) -> bool {
    const BANNED_OPENERS: &[&str] = &[
        "這段",
        "這一段",
        "此段",
        "此處",
        "本段",
        "作者在此",
        "作者在這",
        "this passage",
        "this paragraph",
        "here the author",
        "in this passage",
    ];
    let t = note.trim_start();
    let lower = t.to_lowercase();
    BANNED_OPENERS
        .iter()
        .any(|p| lower.starts_with(&p.to_lowercase()))
}

/// The explanation-level line (講解水位): how much scaffolding the notes
/// assume. `General` is the default and injects nothing (the historical
/// prompt); the two explicit levels steer both selection depth and writing
/// register. Part of `annotation_signature` via the config field.
fn level_block(anno: &AnnotationConfig) -> String {
    match anno.level {
        crate::config::ExplainLevel::General => String::new(),
        crate::config::ExplainLevel::Beginner => "\n\n# 講解水位：入門\n假設讀者對本書的領域沒有基礎：概念一律用日常語言與具體例子講到懂，不預設任何行話；行話出現就順手解釋。選點可以多停在會卡住入門讀者的地方。".to_string(),
        crate::config::ExplainLevel::Insider => "\n\n# 講解水位：內行\n讀者熟悉本書的領域：基礎概念與常識性行話一概不解釋，只補真的查不到、內行也未必知道的東西。選點寧缺勿濫。".to_string(),
    }
}

/// Reader-boundary output check (the Fable rule, machine-enforced): a note may
/// never describe, address or characterise the reader. The profile and anchors
/// steer selection and bridging only — any phrasing that writes the reader into
/// the note text is rejected at parse time exactly like a placeholder leak
/// (N1: the batch retries/splits; N2: the edit verdict is refused, fail-open
/// keeps the original). The list is deliberately conservative: these phrases
/// have no legitimate place in a neutral margin note, so a hit is never a
/// false positive worth keeping.
pub fn note_addresses_reader(note: &str) -> bool {
    const BANNED: &[&str] = &[
        // zh — direct reader-addressing / characterising
        "就像你",
        "跟你一樣",
        "和你一樣",
        "像你這樣",
        "身為讀者",
        "讀者你",
        "的你",
        "你也",
        "你會",
        "你可能",
        "你應該",
        "你我",
        "親愛的讀者",
        // en
        "dear reader",
        "as you ",
        "like you,",
        "you might",
        "you may",
        "you probably",
        "readers like you",
    ];
    let lower = note.to_lowercase();
    BANNED.iter().any(|p| lower.contains(p))
}

/// The preset help angles: `(id, guidance sentence)`. The id set is the CLI /
/// App / signature contract (fixed lowercase); the guidance sentence is what
/// the prompts inject. Canonical order = this table's order.
pub const PRESETS: &[(&str, &str)] = &[
    (
        "terms",
        "專有名詞解釋：術語、專名首次出現時，說明定義、由來與當時的用法。",
    ),
    (
        "history",
        "歷史與時代背景：把事件、制度、風俗放回它們的年代，交代前因與當時的常識。",
    ),
    (
        "author",
        "作者生平與寫作脈絡：作者寫到此處時的處境、動機與思想淵源。",
    ),
    (
        "culture",
        "文化典故與引用：指出出處與原意，以及它在文中承擔的作用。",
    ),
    (
        "characters",
        "人物與關係梳理：這個人是誰、與誰是什麼關係、此刻處境如何。",
    ),
    (
        "concepts",
        "概念白話拆解：把抽象概念用具體、日常的語言講清楚，必要時給一個小例子。",
    ),
    (
        "world",
        "世界連結：把正文接到書外的真實世界（後世發展、別的領域、歷史上的同類事件），只用查證屬實的連結。",
    ),
    (
        "methods",
        "拆方法與原則：在方法、流程、決策原則處停留，交代它的適用條件、代價與上限。",
    ),
    (
        "research",
        "研究輔助：優先補可引用的具體事實與出處、全書的結構骨架，並標出有爭議或需查證之處。",
    ),
];

/// The guidance sentence for one preset id, if known.
pub fn preset_guidance(id: &str) -> Option<&'static str> {
    PRESETS.iter().find(|(pid, _)| *pid == id).map(|(_, g)| *g)
}

/// Canonicalise a raw preset list: known ids only, deduped, in [`PRESETS`]
/// table order. This is the ONLY form the signature and the prompts consume,
/// so id order / duplicates / unknown ids can never change behaviour.
pub fn canonical_presets(raw: &[String]) -> Vec<&'static str> {
    PRESETS
        .iter()
        .filter(|(id, _)| raw.iter().any(|r| r == id))
        .map(|(id, _)| *id)
        .collect()
}

/// The raw entries `canonical_presets` drops — callers surface these as a
/// warning ("unknown id ignored") at the CLI / App boundary.
pub fn unknown_presets(raw: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for r in raw {
        if preset_guidance(r).is_none() && !out.contains(r) {
            out.push(r.clone());
        }
    }
    out
}

/// The injected preset block (empty string when nothing valid is ticked).
/// Placed right after the reader's free text in the selection and writing
/// prompts: the profile says who is reading, the ticks say which kinds of
/// help they explicitly asked for.
fn presets_block(anno: &AnnotationConfig) -> String {
    let picked = canonical_presets(&anno.presets);
    if picked.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = picked
        .iter()
        .filter_map(|id| preset_guidance(id))
        .map(|g| format!("- {g}"))
        .collect();
    format!(
        "\n\n# 讀者勾選的重點方向（明確要求的幫助；選點與取材優先考慮這些角度）\n{}",
        lines.join("\n")
    )
}

/// The reader-profile line for the prompts: the free text, or an honest
/// fallback when the reader picked services/anchors instead of writing prose
/// (the free text is optional by design — recognition beats articulation).
fn profile_line(anno: &AnnotationConfig) -> String {
    let p = anno.reader_profile.trim();
    if p.is_empty() {
        "（讀者未提供文字背景；依下方勾選的服務、講解水位與認知錨判斷。）".to_string()
    } else {
        p.to_string()
    }
}

/// The style paragraph injected into the N1/N2 prompts: the user's style
/// (sanitised — length-capped, whitespace-trimmed) or the engine default.
/// Like `translate::prompt::full_system_prompt`, the surrounding hard rules
/// are non-overridable, so this is a tuning knob, not an escape hatch.
fn style_block(anno: &AnnotationConfig) -> String {
    let style = anno
        .style
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(sanitize_style)
        .unwrap_or_else(|| default_style_for(anno.voice).to_string());
    format!("# 眉批風格（可自訂，不得違反上方硬性規則）\n{style}")
}

fn density_line(d: Density) -> &'static str {
    match d {
        Density::Sparse => "密度：精。只在最關鍵處停留，一章通常 0~2 則。",
        Density::Medium => "密度：適中。一章通常 2~5 則。",
        Density::Rich => {
            "密度：豐。可較常停留（一章通常 5~10 則），但每一則仍必須有實質的背景資訊，不許為了湊數而註。"
        }
    }
}

fn lang_line(anno: &AnnotationConfig) -> String {
    match anno.lang.as_deref().filter(|s| !s.trim().is_empty()) {
        Some(l) => format!("以「{l}」書寫眉批。{}", punctuation_clause(l)),
        None => format!(
            "以與讀者背景文字相同的語言書寫眉批。{}",
            GENERIC_PUNCTUATION_CLAUSE
        ),
    }
}

/// Punctuation instruction for a known note language.
///
/// Models get this wrong per REQUEST, not per book: in one measured run of the
/// same book with the same model, one reader profile out of three wrote every
/// "，" as "," while the other two were clean. A prompt sentence is therefore
/// the first line of defence only; the deterministic fuse is
/// `format::typography::normalize`, applied by the writers.
///
/// Korean deliberately gets no instruction: modern horizontal Korean sets the
/// comma and the full stop as the Latin `,` and `.`, so there is nothing to
/// widen.
fn punctuation_clause(lang_label: &str) -> &'static str {
    let tag = crate::format::epub::lang_attr(lang_label);
    if tag == "zh" || tag.starts_with("zh-") || tag.starts_with("zh_") {
        "標點用中文全形「，。：；！？（）」，不要用半形的 , . : ; ! ? ( )；但引用英文原句時，該英文句子內部維持英文半形標點。"
    } else if tag == "ja" || tag.starts_with("ja-") || tag.starts_with("ja_") {
        "句読点は日本語の全角「、。：；！？（）」を使い、半角の , . : ; ! ? ( ) は使わない。ただし英文を引用する場合、その引用の内部だけは英語の半角記号のままにする。"
    } else {
        GENERIC_PUNCTUATION_CLAUSE
    }
}

/// Used when the note language is not one this module has a specific rule for
/// (including the case where the reader never named one).
const GENERIC_PUNCTUATION_CLAUSE: &str =
    "標點使用該語言自己的標準形式（中文與日文用全形，韓文與西方語言用半形）；引用原文句子時，該引用內部維持原文自己的標點。";

/// The locked content-quality rules — only the NON-NEGOTIABLES (neutrality,
/// never addressing the reader, no meta review, margin-note identity, no
/// re-narration/duplication, no placeholders). Shared verbatim by the writing
/// pass (N1) and the review pass (N2) so an edit can never drift outside the
/// contract. Quality preferences (length target, tone) live in the editable
/// style layer instead — see `DEFAULT_NOTE_STYLE`.
pub fn hard_rules() -> &'static str {
    r#"# 硬性規則（引擎鎖定，不可更動）
1. 眉批內容必須是「中性的背景補充與解釋」：歷史脈絡、事實、術語由來、作者當時的處境、文本結構。具體、可查證。
2. 絕不點破段落與讀者的關係或感受。禁止「就像你…」「跟你一樣」「身為……的你」等任何把讀者寫進內文的說法，也不得引用或改寫讀者背景。讀者背景只用來決定在哪裡停留、選什麼角度、給什麼深度；把「想通」留給讀者。
3. 絕不寫書評或 meta 評論。禁止「這是全書最重要／最常被引用的段落」「這本書值得……的人一讀」等對書本身或段落地位的評價。
4. 眉批以眉批自居，語氣如頁邊小字；不冒充原書內容，不模仿作者口吻，不使用「我」。
5. 不複述、不翻譯段落本身；「已註主題」清單中的主題不得重複解釋。
6. 眉批內不得出現 ⟦…⟧ 佔位符記號，也不得包含任何 HTML 標籤。
7. 絕不劇透。禁止透露讀者在這一段之後、尚未讀到的後文情節、轉折或結局。跨章連結只能「回指」讀者已讀過的前文（可充分展開），不得「預告」後文的具體內容。
8. 連到書外真實世界的事實（後世事件、他人、別領域）必須是可查證的真事實；不確定就不要連，絕不可為了漂亮而杜撰人名、年代或因果。"#
}

/// Pass N0 (plan): sample the whole book → theme map + reader-fit depth
/// positioning. Run once per annotation signature; skipped for mock.
pub fn plan_system(anno: &AnnotationConfig) -> String {
    format!(
        r#"{marker} 你是「Translatus 眉批」的規劃者。眉批是書頁間的硃筆小字：替一位特定讀者，在值得停留的段落補充中性的背景與解釋。
你會收到讀者背景與全書各章開頭的抽樣文字。

# 任務
1. 主題地圖：列出這本書反覆出現、值得註解的概念／人物／事件／術語（8~20 個）。
2. 深度定位：對照讀者背景，指出哪些主題該深入補充、哪些讀者已熟悉可以略過。定位只影響選點與深度；眉批內文永遠中性，不會提及讀者。
3. 一句全書取材指引（哪類段落值得停留）。

# 讀者背景
{profile}{presets}{anchors}{level}

{density}

# 輸出格式（嚴格 JSON，無 markdown 圍欄）
{{"themes":["..."],"focus":"<該深入的方向>","skip":"<可略過的方向>","guidance":"<取材指引>","threads":["<線索：概念/伏筆 — 首現章 → 展開/收束章>"]}}
threads 是全書線索圖：跨章反覆出現、後文才展開或收束的概念、動機、伏筆（4~12 條），每條註明首現與展開的章次。它之後只用來規劃「回指」：讓後文的眉批回頭連結前文；絕不會反過來在前文預告後文。"#,
        marker = N0_MARKER,
        profile = profile_line(anno),
        level = level_block(anno),
        presets = presets_block(anno),
        anchors = anchors_block(anno),
        density = density_line(anno.density),
    )
}

/// Selection pass (per chapter, before any note is written). The model sees the
/// WHOLE chapter compressed (per-paragraph head snippets) and only picks WHERE
/// to stop — id, placement (before/after), angle, priority. It writes no note
/// text; sparsity is then enforced program-side by the chapter cap.
pub fn select_system(
    anno: &AnnotationConfig,
    plan_block: &str,
    topics_block: &str,
    digest_block: &str,
) -> String {
    format!(
        r#"{marker} 你是「Translatus 眉批」的選點者。眉批是書頁間的硃筆小字：替一位特定讀者，在值得停留的段落補充中性的背景與解釋。此刻你只負責「選點」，不寫眉批內文。

# 任務
- 你會收到 JSON 物件 {{"units":[{{"id":<整數>,"text":"<段落開頭，已截斷>"}}],"max_selections":<整數>,"already_selected":[{{"id":<整數>,"angle":"..."}}]}}，units 是本章（或本窗）的連續段落壓縮文。
- 通讀後只挑「最值得停留」的段落，**至多 max_selections 個**；寧缺勿濫，沒有值得註的就少選或不選。
- "already_selected" 是本章先前窗已選的點（唯讀）：不要選相同主題的段落。
- 每個選點給：
  - pos："before"（讀這段之前需要的背景鋪墊）或 "after"（讀完之後的延伸解釋）。依閱讀合適度判斷，不得堆在章節尾。
  - angle：一句話的取材角度（歷史脈絡／術語由來／作者處境／文本結構…）。
  - priority：1~10 整數，越大越重要（超額時程式會依此裁掉低優先者）。
- {density}
- 高價值選點優先：這位讀者因背景差異「必然誤解」或「必然錯過」的段落，以及影響理解全書的關鍵術語／制度首次出現處；正文自己講得清楚的地方不選。
- 名場面／主題高潮不是註點。故事的情感頂點、名句、名場景，讀者自己感受得到；在那裡加註去「解釋它的意義」只會搶走讀者的體會，不要選。
- 每個選點都要能對應這位讀者背景裡的某個具體需求或缺口；純粹的冷知識、與這位讀者無關的補充，不選。
- 跨章連結偏向「回指前文」：若某個洞見要靠拉出後文才成立，就把選點挪到那個洞見**已經出現之後**的段落，改成回頭連結讀者已讀過的前文；不要選一個必須預告後文（劇透）才能註的點。
- 全書眉批計畫裡的「線索圖」給你後文視野：若本章某段正是某條線索**已經展開／收束之後**的位置，優先選它作回指點（angle 註明回指哪條線索）；線索圖僅供選點定位，任何後文內容都不得出現在眉批裡。
- 「已註主題」清單中的主題不得再選。

# 讀者背景（僅用於選點、角度與深度）
{profile}{presets}{anchors}{level}

# 全書眉批計畫
{plan}

# 已註主題（不得重複選）
{topics}

# 前文脈絡（唯讀）
{digest}

# 輸出格式（嚴格 JSON，無 markdown 圍欄）
{{"selections":[{{"id":<原 id>,"pos":"before"|"after","angle":"<取材角度>","priority":<1-10>}}]}}
沒有值得註的段落時輸出 {{"selections":[]}}"#,
        marker = NSEL_MARKER,
        density = density_line(anno.density),
        profile = profile_line(anno),
        level = level_block(anno),
        presets = presets_block(anno),
        anchors = anchors_block(anno),
        plan = plan_block,
        topics = topics_block,
        digest = digest_block,
    )
}

/// Writing pass (per-chapter notes). Runs AFTER the selection pass: every unit
/// it receives was already chosen (with an angle + placement), so its job is to
/// write one note per unit — not to decide sparsity. `plan_block` /
/// `topics_block` / `digest_block` are the rolling-memory injections built by
/// the orchestrator.
pub fn notes_system(
    anno: &AnnotationConfig,
    plan_block: &str,
    topics_block: &str,
    digest_block: &str,
) -> String {
    format!(
        r#"{marker} 你是「Translatus」的眉批作者。眉批是書頁間的硃筆小字：替一位特定讀者，在值得停留的段落補充中性的背景與解釋。選點已完成——你收到的每一段都是被選中的。

# 任務
- 你會收到 JSON 物件 {{"units":[{{"id":<整數>,"text":"...","angle":"<取材角度>","pos":"before"|"after","context_before":"...","context_after":"..."}}]}}。
- 為每個 id 各寫一則眉批，依該段的 angle 取材；context_before / context_after 是相鄰段落的節錄，僅供理解脈絡。
- pos 說明這則眉批的呈現位置：before＝讀該段之前的背景鋪墊（先備知識），after＝讀完之後的延伸解釋。據此拿捏寫法，但內文不得提及位置本身。
- 只有當該段確實沒有實質可補充時，才省略該 id。
- {lang}

{rules}

{style}

# 讀者背景（僅用於角度與深度，內文不得提及）
{profile}{presets}{anchors}{level}

# 全書眉批計畫
{plan}

# 已註主題（不得重複解釋）
{topics}

# 前文脈絡（唯讀）
{digest}

# 輸出格式（嚴格 JSON，無 markdown 圍欄）
{{"notes":[{{"id":<原 id>,"note":"<眉批>"}}],"topics":["<本批新註解的主題，簡短>"]}}"#,
        marker = N1_MARKER,
        lang = lang_line(anno),
        rules = hard_rules(),
        style = style_block(anno),
        profile = profile_line(anno),
        level = level_block(anno),
        presets = presets_block(anno),
        anchors = anchors_block(anno),
        plan = plan_block,
        topics = topics_block,
        digest = digest_block,
    )
}

/// Pass N2 (book-wide review): every note gets keep / edit / drop. The model
/// judges only the `judge` entries; `others` is read-only context so cross-batch
/// duplicates are still visible.
pub fn review_system(anno: &AnnotationConfig) -> String {
    format!(
        r#"{marker} 你是「Translatus」眉批的統一校閱者。全書眉批已寫完；你要去除重複、修正前後不一、削減過密段落。

# 任務
- 你會收到 JSON 物件：{{"reader_profile":"...","judge":[{{"id":<整數>,"chapter":<整數>,"pos":"before"|"after","note":"..."}}],"others":[{{"id":<整數>,"note":"..."}}]}}。
- "judge" 是這一批要你裁決的眉批；"others" 是全書其餘眉批，唯讀，只用來看出重複與矛盾。
- 對 "judge" 中的每一則、且僅對這些 id，輸出一個裁決：
  - keep：保留原文。
  - edit：改寫（修正矛盾、去掉與其他眉批重複的部分）。改寫後仍須遵守下方硬性規則。
  - drop：刪除（同一概念已在更早的眉批解釋過、或無實質內容）。
- 同一概念被解釋多次時：保留最早、資訊最完整的一則，其餘 drop。
- 前後矛盾時：以正確者為準，edit 修正另一則。
- 全書眉批長度過於均勻（清一色同一長度檔）時，把資訊最薄的幾則 edit 成一句話，或 drop。
- 對照下方眉批風格的口吻要求：整組讀起來像導讀講義而非該聲線時，挑判斷句最滿、密度最均勻的幾則 edit——鬆開語氣、砍掉禮貌性收尾，必要時改寫成一句話的短批。
- 不得新增任何 id，不得生成新的眉批。
- pos（before/after 放置位置）是選點階段的決定：edit 只能改內文，不得更動位置。
- {lang}

{rules}

{style}

# 輸出格式（嚴格 JSON 陣列，無 markdown 圍欄）
[{{"id":<id>,"action":"keep"}},{{"id":<id>,"action":"edit","note":"<改寫後>"}},{{"id":<id>,"action":"drop"}}]"#,
        marker = N2_MARKER,
        lang = lang_line(anno),
        rules = hard_rules(),
        style = style_block(anno),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anno(style: Option<&str>) -> AnnotationConfig {
        AnnotationConfig {
            reader_profile: "工程師，想讀懂經濟學經典".into(),
            level: crate::config::ExplainLevel::General,
            anchors: Vec::new(),
            voice: crate::config::NoteVoice::Study,
            lang: Some("繁體中文".into()),
            density: Density::Medium,
            style: style.map(String::from),
            presets: Vec::new(),
        }
    }

    /// The notes must come out in whatever language the caller set — which for a
    /// translate run is the target language. This is the behaviour the product
    /// promises ("translate to 繁體中文 and the notes are in 繁體中文"), and it
    /// had no test locking it.
    #[test]
    fn notes_are_written_in_the_configured_language() {
        for lang in ["English", "繁體中文", "日本語"] {
            let mut a = anno(None);
            a.lang = Some(lang.to_string());
            let sys = notes_system(&a, "plan", "topics", "digest");
            assert!(
                sys.contains(&format!("以「{lang}」書寫眉批")),
                "notes_system for {lang} must instruct that language, got:\n{sys}"
            );
            // And not silently also demand a different one.
            for other in ["English", "繁體中文", "日本語"] {
                if other != lang {
                    assert!(
                        !sys.contains(&format!("以「{other}」書寫眉批")),
                        "{lang} run must not also instruct {other}"
                    );
                }
            }
        }
        // With no language set (annotate-only, no target), notes follow the
        // reader-background language rather than defaulting to anything fixed.
        let mut a = anno(None);
        a.lang = None;
        let sys = notes_system(&a, "plan", "topics", "digest");
        assert!(
            sys.contains("與讀者背景文字相同的語言"),
            "an unset note language must follow the reader background, got:\n{sys}"
        );
    }

    // The editable style layer reaches BOTH prompts that produce note text
    // (N1 writing + N2 review-edit); default applies when unset; the hard
    // rules stay present either way (the style can tune, never replace them).
    #[test]
    fn note_style_injected_into_n1_and_n2() {
        let custom = anno(Some("偏短、口語，每則 80 字以內"));
        for sys in [
            notes_system(&custom, "plan", "topics", "digest"),
            review_system(&custom),
        ] {
            assert!(
                sys.contains("偏短、口語，每則 80 字以內"),
                "custom style in prompt"
            );
            assert!(!sys.contains(DEFAULT_NOTE_STYLE), "custom replaces default");
            assert!(sys.contains("# 硬性規則"), "hard rules always present");
            assert!(sys.contains("# 眉批風格（可自訂"), "style section labelled");
        }
        let default = anno(None);
        for sys in [
            notes_system(&default, "plan", "topics", "digest"),
            review_system(&default),
        ] {
            assert!(sys.contains(DEFAULT_NOTE_STYLE), "default style when unset");
        }
        // Blank style falls back to the default too.
        let blank = anno(Some("   "));
        assert!(notes_system(&blank, "p", "t", "d").contains(DEFAULT_NOTE_STYLE));
        // The selection pass writes no note text — the style does not leak there.
        assert!(!select_system(&custom, "p", "t", "d").contains("偏短、口語"));
    }

    // Sanitisation: an oversized style is hard-capped (same cap as the
    // translation style) so a pasted book chapter can't blow up the prompt.
    #[test]
    fn note_style_is_length_capped() {
        let huge = "長".repeat(crate::translate::prompt::STYLE_MAX_CHARS + 500);
        let sys = notes_system(&anno(Some(&huge)), "p", "t", "d");
        let injected: String = sys
            .split("# 眉批風格（可自訂，不得違反上方硬性規則）\n")
            .nth(1)
            .unwrap()
            .split("\n\n")
            .next()
            .unwrap()
            .to_string();
        assert_eq!(
            injected.chars().count(),
            crate::translate::prompt::STYLE_MAX_CHARS
        );
    }

    // The default style owns the length target now — the hard rules must not
    // (it is a preference, overridable), and the default must state it.
    #[test]
    fn length_target_lives_in_default_style_not_hard_rules() {
        assert!(DEFAULT_NOTE_STYLE.contains(&NOTE_MAX_CHARS.to_string()));
        assert!(!hard_rules().contains("不超過"));
        assert!(!hard_rules().contains(&format!("{NOTE_MAX_CHARS} 字")));
    }

    // Preset guidance reaches the two prompts that pick angles / write notes
    // (selection + writing), right after the reader profile; nothing is
    // injected when no valid preset is ticked; unknown ids are ignored.
    #[test]
    fn preset_guidance_injected_into_selection_and_notes() {
        let mut a = anno(None);
        a.presets = vec!["terms".into(), "history".into(), "bogus".into()];
        let terms_g = preset_guidance("terms").unwrap();
        let history_g = preset_guidance("history").unwrap();
        for sys in [
            select_system(&a, "p", "t", "d"),
            notes_system(&a, "p", "t", "d"),
        ] {
            assert!(sys.contains("# 讀者勾選的重點方向"), "preset block present");
            assert!(sys.contains(terms_g), "terms guidance injected");
            assert!(sys.contains(history_g), "history guidance injected");
            // The block sits after the free-text profile.
            let profile_at = sys.find("工程師，想讀懂經濟學經典").unwrap();
            let block_at = sys.find("# 讀者勾選的重點方向").unwrap();
            assert!(block_at > profile_at, "presets come after the free text");
        }
        // No ticks (or only unknown ids) → no block at all.
        let none = anno(None);
        assert!(!select_system(&none, "p", "t", "d").contains("# 讀者勾選的重點方向"));
        let mut only_bogus = anno(None);
        only_bogus.presets = vec!["bogus".into()];
        assert!(!notes_system(&only_bogus, "p", "t", "d").contains("# 讀者勾選的重點方向"));
    }

    // Cognitive anchors reach every prompt that decides angles or writes text
    // (plan + selection + writing), after the preset block; none listed → no
    // block; canonicalisation caps and dedupes.
    #[test]
    fn anchors_injected_into_plan_selection_and_notes() {
        let mut a = anno(None);
        a.anchors = vec![
            "軟體工程師".into(),
            " 讀過《國富論》 ".into(),
            "軟體工程師".into(),
        ];
        for sys in [
            plan_system(&a),
            select_system(&a, "p", "t", "d"),
            notes_system(&a, "p", "t", "d"),
        ] {
            assert!(sys.contains("# 讀者的認知錨"), "anchor block present");
            assert!(sys.contains("- 軟體工程師"), "anchor listed");
            assert!(sys.contains("- 讀過《國富論》"), "anchor trimmed + listed");
            assert!(sys.contains("類比必須準確"), "accuracy guard present");
        }
        // No anchors → no block.
        let none = anno(None);
        for sys in [
            plan_system(&none),
            select_system(&none, "p", "t", "d"),
            notes_system(&none, "p", "t", "d"),
        ] {
            assert!(!sys.contains("# 讀者的認知錨"));
        }
    }

    #[test]
    fn anchor_canonicalisation_caps_and_dedupes() {
        let raw: Vec<String> = (0..ANCHOR_MAX_COUNT + 5)
            .map(|i| format!("錨{i}"))
            .collect();
        assert_eq!(canonical_anchors(&raw).len(), ANCHOR_MAX_COUNT);
        let raw = vec![
            "  a  ".to_string(),
            "a".to_string(),
            "".to_string(),
            "長".repeat(ANCHOR_MAX_CHARS + 10),
        ];
        let canon = canonical_anchors(&raw);
        assert_eq!(canon[0], "a");
        assert_eq!(canon.len(), 2, "dedup + drop empty");
        assert_eq!(canon[1].chars().count(), ANCHOR_MAX_CHARS, "length cap");
    }

    // The voice register picks the default style paragraph; a custom style
    // overrides either voice; the hard rules are identical in both registers.
    #[test]
    fn voice_selects_default_style_and_custom_style_wins() {
        use crate::config::NoteVoice;
        let mut companion = anno(None);
        companion.voice = NoteVoice::Companion;
        for sys in [
            notes_system(&companion, "p", "t", "d"),
            review_system(&companion),
        ] {
            assert!(
                sys.contains(COMPANION_NOTE_STYLE),
                "companion style in prompt"
            );
            assert!(!sys.contains(DEFAULT_NOTE_STYLE), "study style absent");
            assert!(sys.contains("# 硬性規則"), "hard rules unchanged");
        }
        // Study (default) keeps the historical paragraph.
        assert!(notes_system(&anno(None), "p", "t", "d").contains(DEFAULT_NOTE_STYLE));
        // A custom style beats the companion voice too.
        let mut custom = anno(Some("每則 80 字以內"));
        custom.voice = NoteVoice::Companion;
        let sys = notes_system(&custom, "p", "t", "d");
        assert!(sys.contains("每則 80 字以內"));
        assert!(!sys.contains(COMPANION_NOTE_STYLE));
    }

    /// Both writing passes carry a punctuation instruction matched to the note
    /// language, because a model that types "," for "，" does it for a whole
    /// request. This is the FIRST line of defence only: the deterministic fuse
    /// is `format::typography::normalize` in the writers, which is why none of
    /// this text enters `annotation_signature`.
    #[test]
    fn the_punctuation_clause_follows_the_note_language() {
        let with_lang = |l: Option<&str>| {
            let mut a = anno(None);
            a.lang = l.map(String::from);
            (notes_system(&a, "p", "t", "d"), review_system(&a))
        };

        for label in ["繁體中文", "简体中文", "中文"] {
            let (notes, review) = with_lang(Some(label));
            for sys in [&notes, &review] {
                assert!(sys.contains("標點用中文全形"), "{label}: {sys}");
                assert!(
                    sys.contains("引用英文原句"),
                    "quotations protected: {label}"
                );
            }
        }

        let (notes, review) = with_lang(Some("日本語"));
        for sys in [&notes, &review] {
            assert!(sys.contains("句読点は日本語の全角"), "ja clause: {sys}");
            assert!(
                !sys.contains("標點用中文全形"),
                "ja must not get the zh clause"
            );
        }

        // Korean sets the comma and the full stop as the Latin characters, so
        // it must never be told to widen them.
        let (notes, _) = with_lang(Some("한국어"));
        assert!(
            !notes.contains("標點用中文全形") && !notes.contains("句読点は日本語の全角"),
            "Korean must never be ordered to widen its punctuation"
        );
        assert!(notes.contains(GENERIC_PUNCTUATION_CLAUSE));
        assert!(
            GENERIC_PUNCTUATION_CLAUSE.contains("韓文與西方語言用半形"),
            "the generic clause has to say Korean stays halfwidth"
        );

        // No language named: a generic clause that still protects quotations.
        let (notes, _) = with_lang(None);
        assert!(notes.contains(GENERIC_PUNCTUATION_CLAUSE));
    }

    // 具體開頭律, machine-enforced: generic framing openers are rejected;
    // concrete openers (and quoted text) pass.
    #[test]
    fn generic_opener_detection() {
        for bad in [
            "這段描寫了 Laura 的內心轉折。",
            "  此處作者使用了自由間接引語。",
            "This passage shows the class divide.",
        ] {
            assert!(note_opens_generic(bad), "must flag: {bad}");
        }
        for ok in [
            "「And after all」在英語口語裡是轉折語氣。",
            "1931 年 Adams 才鑄出「美國夢」一詞。",
            "薰衣草那一捏，Mansfield 用身體動作替代了心理分析。",
        ] {
            assert!(!note_opens_generic(ok), "must pass: {ok}");
        }
    }

    // The Fable rule, machine-enforced: reader-addressing phrasings are
    // detected; neutral notes about the BOOK (including ones about the author)
    // pass.
    #[test]
    fn reader_addressing_detection() {
        for bad in [
            "就像你寫程式時拆函式，斯密把製針拆成十八道工序。",
            "身為讀者，你會發現這一段其實在鋪墊。",
            "跟你一樣，富蘭克林也常常自我懷疑。",
            "Dear reader, this is where the theme turns.",
            "As you know, the term was coined six years later.",
        ] {
            assert!(note_addresses_reader(bad), "must flag: {bad}");
        }
        for ok in [
            "「美國夢」一詞比本書晚六年出生：1931 年 James Truslow Adams 才鑄出它。",
            "斯密在第五卷自己警告了分工的代價：重複勞動使人「變得愚鈍」。",
            "作者寫到此處時正流亡海外，靠友人接濟度日。",
            "妙在「逆水行舟」四字：前文的船正是順流而下。",
        ] {
            assert!(!note_addresses_reader(ok), "must pass: {ok}");
        }
    }

    // The explanation level steers every prompt that decides depth; General
    // (default) injects nothing; and "services ticked, nothing written" is a
    // first-class configuration (honest fallback line replaces the profile).
    #[test]
    fn explain_level_injected_and_profile_optional() {
        use crate::config::ExplainLevel;
        let mut a = anno(None);
        a.reader_profile = String::new();
        a.presets = vec!["world".into()];
        a.level = ExplainLevel::Beginner;
        for sys in [
            plan_system(&a),
            select_system(&a, "p", "t", "d"),
            notes_system(&a, "p", "t", "d"),
        ] {
            assert!(sys.contains("# 講解水位：入門"), "beginner level injected");
            assert!(
                sys.contains("讀者未提供文字背景"),
                "empty profile must degrade to the honest fallback line"
            );
        }
        let mut ins = anno(None);
        ins.level = ExplainLevel::Insider;
        assert!(notes_system(&ins, "p", "t", "d").contains("# 講解水位：內行"));
        // Default level injects no level section; a written profile → no fallback.
        let plain = anno(None);
        let sys = notes_system(&plain, "p", "t", "d");
        assert!(!sys.contains("# 講解水位"));
        assert!(!sys.contains("讀者未提供文字背景"));
        assert!(sys.contains("工程師，想讀懂經濟學經典"));
    }

    // The three v7 service additions are real members of the preset contract:
    // guidance exists, canonicalisation accepts them, prompts inject them.
    #[test]
    fn service_menu_covers_world_methods_research() {
        for id in ["world", "methods", "research"] {
            assert!(preset_guidance(id).is_some(), "{id} must have guidance");
        }
        let mut a = anno(None);
        a.presets = vec!["world".into(), "research".into()];
        let sys = select_system(&a, "p", "t", "d");
        assert!(sys.contains("世界連結"), "world guidance injected");
        assert!(sys.contains("研究輔助"), "research guidance injected");
    }

    // The canonicalisation contract the signature relies on.
    #[test]
    fn preset_canonicalisation_and_unknown_reporting() {
        let raw = vec![
            "history".to_string(),
            "terms".to_string(),
            "terms".to_string(),
            "bogus".to_string(),
        ];
        // Table order, deduped, unknown dropped.
        assert_eq!(canonical_presets(&raw), vec!["terms", "history"]);
        assert_eq!(unknown_presets(&raw), vec!["bogus".to_string()]);
        // The full known set round-trips in table order.
        let all: Vec<String> = PRESETS.iter().map(|(id, _)| id.to_string()).collect();
        assert_eq!(canonical_presets(&all).len(), PRESETS.len());
        assert!(unknown_presets(&all).is_empty());
    }
}
