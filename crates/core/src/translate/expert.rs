//! Expert (multi-pass) translation. The algorithm — not the prompt — is what
//! separates it from sentence level:
//!
//!   Pass 0  Pre-scan  → glossary (proper-noun records, first-occurrence locked)
//!                       + book-wide style guide
//!   Pass 1  Translate → per chapter, with glossary hard-constraint + style guide
//!                       + rolling bilingual summary injected
//!   Pass 2  Reflect   → one revision round over the freshly drafted segments
//!   Pass 3  Consistency (program-side, see `consistency_report`)
//!
//! Mirrors DelTA's structured memory + Andrew Ng's reflection + TransAgents'
//! translation guide. The LLM-driven aux passes (0/2 + summary) need a real
//! model, so they are skipped for the offline `mock` provider.

use super::{glossary_sub, translate_unit_map, Stats};
use crate::config::{Level, ProviderKind, TranslateConfig};
use crate::document::{Book, Chapter};
use crate::error::Result;
use crate::format::placeholder;
use crate::llm::{ChatMessage, CompletionRequest, Provider};
use crate::memory::{ExpertMemory, GlossaryEntry};
use crate::translate::prompt;
use crate::validate;
use serde::Deserialize;
use serde_json::json;
use std::collections::{BTreeMap, HashMap};

const DEFAULT_EXPERT_GUIDE: &str =
    "維持忠實與自然；全書語氣、人稱、稱謂一致；專有名詞統一譯法；保留原作文體。";

const SCAN_CHARS_PER_CHAPTER: usize = 2500;

fn strip_placeholders(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tok = false;
    for c in s.chars() {
        match c {
            prompt_open if prompt_open == crate::format::placeholder::OPEN => in_tok = true,
            prompt_close if prompt_close == crate::format::placeholder::CLOSE => in_tok = false,
            c if !in_tok => out.push(c),
            _ => {}
        }
    }
    out
}

fn chapter_plain_text(chapter: &Chapter, cap: usize) -> String {
    let mut s = String::new();
    for seg in &chapter.segments {
        s.push_str(&strip_placeholders(&seg.source));
        s.push('\n');
        if s.chars().count() >= cap {
            break;
        }
    }
    s
}

// ── Pass 0: pre-scan ───────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
struct RawTerm {
    #[serde(default)]
    term: String,
    #[serde(default)]
    translation: String,
    #[serde(default, rename = "type")]
    kind: String,
}

#[derive(Deserialize, Default)]
struct ExtractResult {
    #[serde(default)]
    glossary: Vec<RawTerm>,
    #[serde(default)]
    style_guide: Vec<String>,
}

fn extract_json_object(s: &str) -> Option<String> {
    let start = s.find('{')?;
    let end = s.rfind('}')?;
    if end > start {
        Some(s[start..=end].to_string())
    } else {
        None
    }
}

fn extract_system(cfg: &TranslateConfig) -> String {
    format!(
        r#"你在為一本書建立「翻譯前的術語表與風格指南」。只閱讀，不翻譯正文。
任務：
1. 列出專有名詞（人名 / 地名 / 組織 / 作品名）與反覆出現的關鍵術語。
2. 對每個項目，依目標語言「{lang}」給出建議譯法，並標註類型（person/place/org/term）。
3. 判定文體、語氣、目標讀者、人稱與稱謂習慣，寫成 3~6 條風格指南。
僅輸出 JSON（無 markdown 圍欄）：
{{"glossary":[{{"term":"...","translation":"...","type":"..."}}],"style_guide":["...","..."]}}"#,
        lang = cfg.target_lang
    )
}

/// Scan the whole book → glossary + style guide. Returns the memory and the
/// tokens it cost.
pub async fn prescan(
    provider: &Provider,
    cfg: &TranslateConfig,
    book: &Book,
) -> Result<(ExpertMemory, Stats)> {
    let mut mem = ExpertMemory {
        style_guide: DEFAULT_EXPERT_GUIDE.to_string(),
        ..Default::default()
    };
    let mut stats = Stats::default();

    // The echo provider can't do extraction meaningfully.
    if cfg.provider == ProviderKind::Mock {
        return Ok((mem, stats));
    }

    let system = extract_system(cfg);
    for (ci, chapter) in book.chapters.iter().enumerate() {
        let text = chapter_plain_text(chapter, SCAN_CHARS_PER_CHAPTER);
        if text.trim().is_empty() {
            continue;
        }
        let req = CompletionRequest {
            system: system.clone(),
            messages: vec![ChatMessage::user(text)],
            temperature: 0.1,
        };
        let resp = match provider.complete_retrying(&req, 1).await {
            Ok(r) => r,
            Err(_) => continue, // best-effort; a failed scan must not abort translation
        };
        stats.tokens_in += resp.tokens_in;
        stats.tokens_out += resp.tokens_out;

        let parsed: ExtractResult = extract_json_object(&resp.text)
            .and_then(|j| serde_json::from_str(&j).ok())
            .unwrap_or_default();

        mem.add_terms(parsed.glossary.into_iter().map(|t| GlossaryEntry {
            source: t.term,
            target: t.translation,
            kind: if t.kind.is_empty() {
                "term".into()
            } else {
                t.kind
            },
            first_chapter: ci,
        }));

        if mem.style_guide == DEFAULT_EXPERT_GUIDE && !parsed.style_guide.is_empty() {
            mem.style_guide = parsed.style_guide.join("；");
        }
    }
    Ok((mem, stats))
}

// ── Pass 1 + 2: translate a chapter with memory, then reflect ──────────────

fn glossary_block(matched: &[&GlossaryEntry]) -> String {
    if matched.is_empty() {
        "（本段無適用術語）".to_string()
    } else {
        matched
            .iter()
            .map(|g| format!("- {} → {}", g.source, g.target))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn translate_system(
    cfg: &TranslateConfig,
    mem: &ExpertMemory,
    matched: &[&GlossaryEntry],
) -> String {
    let style = cfg
        .custom_prompt
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| prompt::default_style(Level::Expert));
    let summary = if mem.summary.trim().is_empty() {
        "（尚無前文）".to_string()
    } else {
        mem.summary.clone()
    };
    format!(
        "{head}\n\n# 翻譯指南（風格 / 語氣 / 讀者）\n{guide}\n\n# 術語表（必須全書統一使用以下譯法）\n{gloss}\n\n# 前文摘要（唯讀，維持連貫）\n{summary}\n\n# 翻譯風格（可自訂）\n{style}\n\n{tail}",
        head = prompt::locked_head(&cfg.target_lang),
        guide = mem.style_guide,
        gloss = glossary_block(matched),
        summary = summary,
        style = style,
        tail = prompt::locked_tail(),
    )
}

fn reflect_system(cfg: &TranslateConfig, mem: &ExpertMemory, matched: &[&GlossaryEntry]) -> String {
    format!(
        r#"你是資深文學編輯，負責校訂翻譯草稿。你會收到一個 JSON 物件，內含 "sentences" 陣列，每個元素包含：
- "source"：原文，供你檢查語意、漏譯、增譯與文體。
- "draft"：待校訂的「{lang}」譯文草稿。

請逐句對照 source 與 draft 後改進：修正術語不一致、代名詞 / 稱謂、語氣偏離、漏譯、誤譯、增譯、過度翻譯、翻譯腔，使其更自然且符合風格指南與術語表。

# 硬性規則（引擎鎖定）
1. 一進一出，不得合併 / 拆分 / 新增 / 遺漏；每個 id 對應正確。
2. 原樣保留所有佔位符（⟦n⟧、⟦Cn⟧、⟦Gn⟧）：數量、編號、配對不可更動。
3. 只輸出 JSON 陣列：[{{"id": <id>, "translation": "<改進後的譯文>"}}]

# 翻譯指南
{guide}

# 術語表（務必統一）
{gloss}"#,
        lang = cfg.target_lang,
        guide = mem.style_guide,
        gloss = glossary_block(matched),
    )
}

#[derive(Debug, Clone)]
struct ReflectUnit {
    id: u64,
    source: String,
    draft: String,
}

fn reflect_payload(batch: &[ReflectUnit]) -> String {
    let sentences: Vec<_> = batch
        .iter()
        .map(|u| json!({ "id": u.id, "source": u.source, "draft": u.draft }))
        .collect();
    json!({ "sentences": sentences }).to_string()
}

fn parse_reflect_response(raw: &str, batch: &[ReflectUnit]) -> Result<HashMap<u64, String>> {
    let drafts: HashMap<u64, String> = batch.iter().map(|u| (u.id, u.draft.clone())).collect();
    validate::parse_and_check(raw, &drafts)
}

async fn reflect_unit_map(
    provider: &Provider,
    cfg: &TranslateConfig,
    units: Vec<ReflectUnit>,
    system: &str,
    stats: &mut Stats,
) -> HashMap<u64, String> {
    let mut work: Vec<Vec<ReflectUnit>> = Vec::new();
    let mut cur: Vec<ReflectUnit> = Vec::new();
    let mut cur_tokens = 0usize;
    for unit in units {
        let chars = unit.source.chars().count() + unit.draft.chars().count();
        let t = (chars as f32 / 2.5).ceil() as usize;
        if !cur.is_empty()
            && (cur.len() >= cfg.max_batch_sentences || cur_tokens + t > cfg.max_chunk_tokens)
        {
            work.push(std::mem::take(&mut cur));
            cur_tokens = 0;
        }
        cur.push(unit);
        cur_tokens += t;
    }
    if !cur.is_empty() {
        work.push(cur);
    }

    let mut results: HashMap<u64, String> = HashMap::new();
    while let Some(batch) = work.pop() {
        match reflect_once(provider, &batch, system, stats).await {
            Ok(map) => {
                stats.units_translated += map.len();
                results.extend(map);
            }
            Err(_) if batch.len() > 1 => {
                let mid = batch.len() / 2;
                let (a, b) = batch.split_at(mid);
                work.push(a.to_vec());
                work.push(b.to_vec());
            }
            Err(e) => {
                let unit = &batch[0];
                results.insert(unit.id, unit.draft.clone());
                stats.units_failed += 1;
                stats.sample_error.get_or_insert_with(|| e.to_string());
            }
        }
    }
    results
}

async fn reflect_once(
    provider: &Provider,
    batch: &[ReflectUnit],
    system: &str,
    stats: &mut Stats,
) -> Result<HashMap<u64, String>> {
    let payload = reflect_payload(batch);
    let req = CompletionRequest {
        system: system.to_string(),
        messages: vec![ChatMessage::user(payload)],
        temperature: 0.2,
    };

    let resp = provider.complete_retrying(&req, 2).await?;
    stats.tokens_in += resp.tokens_in;
    stats.tokens_out += resp.tokens_out;

    parse_reflect_response(&resp.text, batch)
}

fn summarize_system(cfg: &TranslateConfig) -> String {
    format!(
        r#"更新「雙語滾動摘要」。你會收到：舊摘要、本章原文、本章譯文（{lang}）。
輸出一段 ≤200 字的摘要，保留：情節推進、人物關係變化、新登場的人 / 地 / 術語及其譯法。丟棄細節。只輸出摘要文字，不要任何前後綴。"#,
        lang = cfg.target_lang
    )
}

/// Translate one chapter at expert level: glossary/style/summary-constrained
/// draft, then one reflection round over the freshly drafted segments, then a
/// rolling-summary update. Mutates `mem` (summary).
pub async fn translate_chapter_expert(
    provider: &Provider,
    cfg: &TranslateConfig,
    chapter: &mut Chapter,
    mem: &mut ExpertMemory,
    on_batch: super::PairSink<'_>,
) -> Result<Stats> {
    let mut stats = Stats::default();

    let plain = chapter_plain_text(chapter, usize::MAX);
    let matched = mem.matched(&plain);

    // Build the units to translate with deterministic glossary substitution:
    // locked terms in the source become ⟦Gn⟧ sentinels (which the LLM won't
    // touch); the sentinel→locked-target map is folded into each segment's
    // placeholders after drafting so write-back restores the locked translation.
    let mut pending: Vec<(u64, String)> = Vec::new();
    let mut sub_map: HashMap<u64, BTreeMap<String, String>> = HashMap::new();
    for seg in &chapter.segments {
        if seg.target.is_some() {
            continue;
        }
        let seg_matched: Vec<&GlossaryEntry> = matched
            .iter()
            .copied()
            .filter(|g| seg.source.contains(&g.source))
            .collect();
        let r = glossary_sub::substitute(&seg.source, &seg_matched);
        if !r.sentinels.is_empty() {
            sub_map.insert(seg.block_index as u64, r.sentinels);
        }
        pending.push((seg.block_index as u64, r.substituted));
    }
    if pending.is_empty() {
        return Ok(stats);
    }

    // Pass 1: draft.
    let system = translate_system(cfg, mem, &matched);
    let drafted =
        translate_unit_map(provider, cfg, pending, &system, &mut stats, Some(on_batch)).await;
    let fresh_ids: Vec<u64> = drafted.keys().copied().collect();
    for seg in &mut chapter.segments {
        if seg.target.is_none() {
            if let Some(t) = drafted.get(&(seg.block_index as u64)) {
                seg.target = Some(t.clone());
                // Fold sentinels in — for both success and source-fallback results,
                // so a fallback never leaves a raw ⟦Gn⟧ in the output.
                if let Some(sents) = sub_map.get(&(seg.block_index as u64)) {
                    for (k, v) in sents {
                        seg.placeholders.insert(k.clone(), v.clone());
                    }
                }
            }
        }
    }

    // Pass 2: reflect over freshly drafted segments (real providers only).
    if cfg.provider != ProviderKind::Mock && !fresh_ids.is_empty() {
        let drafts: Vec<ReflectUnit> = chapter
            .segments
            .iter()
            .filter(|s| fresh_ids.contains(&(s.block_index as u64)))
            .filter_map(|s| {
                s.target.clone().map(|draft| ReflectUnit {
                    id: s.block_index as u64,
                    source: s.source.clone(),
                    draft,
                })
            })
            .collect();
        let rsystem = reflect_system(cfg, mem, &matched);
        let improved = reflect_unit_map(provider, cfg, drafts, &rsystem, &mut stats).await;
        for seg in &mut chapter.segments {
            if let Some(t) = improved.get(&(seg.block_index as u64)) {
                seg.target = Some(t.clone());
            }
        }
    }

    // Bake locked glossary translations into the draft so the CACHED target is
    // self-contained (no runtime-only ⟦Gn⟧). Must run after reflection (which
    // needs the sentinels for alignment) and before the chapter is cached.
    bake_glossary_sentinels(chapter, &sub_map);

    // Rolling summary update (real providers only).
    if cfg.provider != ProviderKind::Mock {
        let src = chapter_plain_text(chapter, 4000);
        let tgt: String = chapter
            .segments
            .iter()
            .filter_map(|s| {
                // Restore so the summary sees locked glossary translations (⟦Gn⟧),
                // not the raw sentinels; then drop any leftover tokens.
                s.target
                    .as_ref()
                    .map(|t| strip_placeholders(&placeholder::restore(t, &s.placeholders)))
            })
            .collect::<Vec<_>>()
            .join("\n");
        let payload = format!(
            "【舊摘要】\n{}\n\n【本章原文】\n{}\n\n【本章譯文】\n{}",
            mem.summary, src, tgt
        );
        let req = CompletionRequest {
            system: summarize_system(cfg),
            messages: vec![ChatMessage::user(payload)],
            temperature: 0.2,
        };
        if let Ok(resp) = provider.complete_retrying(&req, 1).await {
            stats.tokens_in += resp.tokens_in;
            stats.tokens_out += resp.tokens_out;
            let s = resp.text.trim();
            if !s.is_empty() {
                mem.summary = s.chars().take(1200).collect();
            }
        }
    }

    Ok(stats)
}

// ── Pass 3: consistency report (program-side) ──────────────────────────────

/// For each locked glossary term, flag segments whose source contains the term
/// but whose translation does not contain the locked target translation.
pub fn consistency_report(book: &Book, mem: &ExpertMemory) -> Vec<String> {
    let mut out = Vec::new();
    for g in &mem.glossary {
        if g.source.trim().is_empty() || g.target.trim().is_empty() {
            continue;
        }
        let mut misses = 0usize;
        for chapter in &book.chapters {
            for seg in &chapter.segments {
                let src = strip_placeholders(&seg.source);
                if src.contains(&g.source) {
                    if let Some(t) = &seg.target {
                        // Restore sentinels first; with hard-replace the locked
                        // target lives in placeholders, not the raw ⟦Gn⟧ target.
                        let restored = placeholder::restore(t, &seg.placeholders);
                        if !restored.contains(&g.target) {
                            misses += 1;
                        }
                    }
                }
            }
        }
        if misses > 0 {
            out.push(format!(
                "術語「{} → {}」有 {} 處未一致套用",
                g.source, g.target, misses
            ));
        }
        if out.len() >= 50 {
            break;
        }
    }
    out
}

/// Replace each `⟦Gn⟧` glossary sentinel in a segment's drafted target with its
/// locked translation, leaving DOM tokens (`⟦n⟧`/`⟦Cn⟧`) intact for write-time
/// restore. Makes the cached target self-contained so resume / cache-only
/// re-render cannot leak raw sentinels. `sub_map` is block_index → (Gn → target).
fn bake_glossary_sentinels(
    chapter: &mut Chapter,
    sub_map: &HashMap<u64, BTreeMap<String, String>>,
) {
    for seg in &mut chapter.segments {
        if let Some(sents) = sub_map.get(&(seg.block_index as u64)) {
            if let Some(t) = &seg.target {
                seg.target = Some(placeholder::restore(t, sents));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Chapter, Format, Segment};

    fn seg(idx: usize, source: &str, target: &str) -> Segment {
        let mut s = Segment::new(idx, source.into(), Default::default());
        s.target = Some(target.into());
        s
    }

    /// Regression for the resume / cache-only ⟦Gn⟧ leak: after baking, the
    /// cached target must contain the locked term, never the raw sentinel, while
    /// DOM tokens (⟦1⟧) survive for write-time restore. See 2026-06-06 fix.
    #[test]
    fn bake_glossary_makes_target_self_contained() {
        let mut chapter = Chapter {
            spine_index: 0,
            href: "c".into(),
            title: None,
            segments: vec![seg(0, "Ahab ⟦1⟧spoke⟦/1⟧", "⟦G0⟧ ⟦1⟧說話⟦/1⟧")],
            apparatus: false,
        };
        let mut sub_map: HashMap<u64, BTreeMap<String, String>> = HashMap::new();
        let mut sents = BTreeMap::new();
        sents.insert("G0".to_string(), "亞哈".to_string());
        sub_map.insert(0, sents);

        bake_glossary_sentinels(&mut chapter, &sub_map);

        let t = chapter.segments[0].target.as_ref().unwrap();
        assert!(!t.contains("⟦G"), "glossary sentinel leaked: {t}");
        assert!(t.contains("亞哈"), "locked term not baked in: {t}");
        assert!(
            t.contains("⟦1⟧"),
            "DOM token must survive for write-time restore: {t}"
        );
    }

    #[test]
    fn consistency_flags_term_misuse() {
        let book = Book {
            format: Format::Epub,
            title: None,
            chapters: vec![Chapter {
                spine_index: 0,
                href: "c1".into(),
                title: None,
                segments: vec![
                    seg(0, "Ahab spoke.", "亞哈說話了。"),  // consistent
                    seg(1, "Ahab left.", "阿哈伯離開了。"), // inconsistent (wrong target)
                ],
                apparatus: false,
            }],
        };
        let mut mem = ExpertMemory::default();
        mem.add_terms([GlossaryEntry {
            source: "Ahab".into(),
            target: "亞哈".into(),
            kind: "person".into(),
            first_chapter: 0,
        }]);
        let report = consistency_report(&book, &mem);
        assert_eq!(report.len(), 1);
        assert!(report[0].contains("Ahab"));
    }

    #[test]
    fn strip_placeholders_removes_tokens() {
        assert_eq!(strip_placeholders("一⟦1⟧紅⟦/1⟧車⟦C2⟧"), "一紅車");
    }

    #[test]
    fn reflect_payload_includes_source_and_draft() {
        let payload = reflect_payload(&[ReflectUnit {
            id: 7,
            source: "Ahab spoke.".into(),
            draft: "亞哈說話了。".into(),
        }]);
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        let item = &v["sentences"][0];
        assert_eq!(item["id"], 7);
        assert_eq!(item["source"], "Ahab spoke.");
        assert_eq!(item["draft"], "亞哈說話了。");
    }

    #[test]
    fn reflect_response_validation_uses_draft_placeholders() {
        let batch = [ReflectUnit {
            id: 1,
            source: "Hello ⟦1⟧world⟦/1⟧.".into(),
            draft: "你好⟦1⟧世界⟦/1⟧。".into(),
        }];
        let ok = r#"[{"id":1,"translation":"您好⟦1⟧世界⟦/1⟧。"}]"#;
        assert!(parse_reflect_response(ok, &batch).is_ok());

        let missing = r#"[{"id":1,"translation":"您好世界。"}]"#;
        assert!(parse_reflect_response(missing, &batch).is_err());
    }
}
