//! System prompts, split into a **locked** part (the engine's hard rules — no
//! split/merge, preserve placeholders, strict JSON) and an **editable style**
//! part (the user's tone/wording/audience guidance). The UI may view the locked
//! part read-only and edit only the style part; the core has no setter for the
//! hard rules, so the user physically cannot break alignment/format. This is the
//! resolution of "view + edit the system prompt" vs "don't let users break the
//! engine".

use crate::config::{Level, TranslateConfig};

/// Default editable style paragraph for a level. "Restore default" returns this.
pub fn default_style(level: Level) -> &'static str {
    match level {
        Level::Sentence => {
            "忠實、自然、通順；不增譯、不刪減、不改寫語意。保留原文的語氣與標點習慣。"
        }
        Level::Expert => {
            "在忠實的前提下追求全書用詞一致與閱讀連貫；語氣貼合原作文體，專有名詞全書統一譯法。"
        }
    }
}

/// Locked head: role + task + hard rules. `lang_display` is the real target
/// language at runtime, or a `{目標語言}` placeholder for the UI preview.
pub fn locked_head(lang_display: &str) -> String {
    format!(
        r#"你是一位專業的書籍譯者，將文本翻譯成「{lang}」。

# 任務
- 你會收到一個 JSON 物件，內含 "sentences" 陣列，每個元素是一個待翻句子，含唯一整數 id。
- 對「每一個」句子產生譯文，並以相同 id 對齊回傳。

# 硬性規則（引擎鎖定，不可更動 — 改動會破壞句子對齊與 EPUB 格式）
1. 一進一出：輸入幾句、就輸出幾句。不得合併、不得拆分、不得新增、不得遺漏任何句子。
2. 每個 id 必須原樣保留，譯文必須對應正確的 id。
3. 原樣保留所有佔位符（例如 ⟦1⟧…⟦/1⟧ 或 ⟦C1⟧）：數量、編號、配對皆不可更動，僅可隨語序移動其位置。
4. 不要輸出任何解釋、註解、原文或額外文字。只輸出 JSON 陣列。
5. 保留專有名詞、程式碼、URL、數字、數學式原樣（除非該語言有通用譯名）。
6. 嚴格使用「{lang}」的標準字形、用語與標點：目標為繁體中文時一律繁體字（禁止混入簡體字）、台灣慣用語與全形標點；目標為简体中文時一律簡體字。整本書字形必須一致。
7. 同一專有名詞（人名、地名）全書譯法一致：若決定音譯就全程同一音譯，不得時而音譯時而保留原文。"#,
        lang = lang_display
    )
}

/// Locked tail: output format contract.
pub fn locked_tail() -> &'static str {
    "# 輸出格式（嚴格 JSON，無 markdown 圍欄）\n[{\"id\": <原 id>, \"translation\": \"<譯文>\"}, ...]"
}

/// Hard length cap on a user style paragraph.
pub const STYLE_MAX_CHARS: usize = 4000;

/// Sanitise the user style: enforce a length cap. Even if the UI is bypassed the
/// engine still wraps it with the locked rules, so this is a soft guard.
pub fn sanitize_style(style: &str) -> String {
    let s = style.trim();
    if s.chars().count() > STYLE_MAX_CHARS {
        s.chars().take(STYLE_MAX_CHARS).collect()
    } else {
        s.to_string()
    }
}

/// Assemble the full system prompt: locked head + editable style + locked tail.
/// `style` of `None`/empty falls back to the level default.
pub fn full_system_prompt(level: Level, lang_display: &str, style: Option<&str>) -> String {
    let style = style
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(sanitize_style)
        .unwrap_or_else(|| default_style(level).to_string());
    format!(
        "{head}\n\n# 翻譯風格（可自訂）\n{style}\n\n{tail}",
        head = locked_head(lang_display),
        style = style,
        tail = locked_tail(),
    )
}

/// The real prompt used at translation time.
pub fn sentence_system(cfg: &TranslateConfig) -> String {
    full_system_prompt(cfg.level, &cfg.target_lang, cfg.custom_prompt.as_deref())
}
