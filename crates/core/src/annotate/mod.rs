//! Reader-personalised annotations (眉批) — the engine's second core capability.
//!
//!   Pass N0    Plan   → sample every chapter's opening, build a book-wide theme
//!                       map + reader-fit depth positioning (cached in meta).
//!   Pass N-sel Select → per chapter, the WHOLE chapter compressed (per-segment
//!                       head snippets, windowed when huge) goes to the model,
//!                       which only picks WHERE to stop: `{id, pos, angle,
//!                       priority}`. A program-side hard cap (density × chapter
//!                       size) then trims by priority — sparsity is an
//!                       architectural guarantee, not model self-discipline.
//!   Pass N1    Notes  → the selected segments only, batched with their angle,
//!                       placement and neighbouring context. Rolling memory —
//!                       noted topics + a book digest — prevents upstream dupes.
//!   Pass N2    Review → one book-wide unification pass: program-side exact
//!                       dedupe, then batched keep/edit/drop by the model
//!                       (placement is selection's decision; edits keep it).
//!
//! Every per-segment result — including the deliberate "no note here" (a skip
//! note) — is cached under the annotation signature, so an interrupted run
//! resumes at zero token cost. Orchestrated from `translate::run`.

pub mod prompt;

use crate::config::{AnnotationConfig, Density, ProviderKind, TranslateConfig};
use crate::document::{Book, Chapter, Note, NotePos};
use crate::error::Result;
use crate::format::placeholder;
use crate::llm::{ChatMessage, CompletionRequest, Provider};
use crate::translate::{has_visible_text, Stats};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, HashSet};

/// Per-chapter head-sampling budget for the plan pass (chars of visible text).
const PLAN_CHARS_PER_CHAPTER: usize = 900;
/// Per-chapter mid-sampling budget for the plan pass: a slice from the middle
/// of the chapter, so the thread map sees how chapters DEVELOP (a payoff
/// rarely sits in the opening lines).
const PLAN_MID_CHARS: usize = 400;
/// Whole-book cap for the plan sample (chars).
const PLAN_TOTAL_CHARS: usize = 32_000;
/// Notes judged per review call. The rest of the book's notes ride along as
/// read-only context so cross-batch duplicates stay visible.
const REVIEW_BATCH: usize = 60;
/// Context notes are truncated to this many chars in the review payload.
const REVIEW_CONTEXT_CHARS: usize = 80;
/// Rolling book digest cap (chars) — enough to anchor "what came before".
const DIGEST_CAP_CHARS: usize = 1600;
/// Rolling noted-topics cap (entries).
const TOPICS_CAP: usize = 120;
/// Per-segment head snippet sent to the selection pass (chars).
const SELECT_SNIPPET_CHARS: usize = 200;
/// Snippet-char budget of one selection window; oversized chapters split into
/// windows, later windows carrying the earlier windows' picks read-only.
const SELECT_WINDOW_CHARS: usize = 12_000;
/// Neighbour-context snippet attached to each selected unit in the writing
/// pass (chars per side).
const GEN_CONTEXT_CHARS: usize = 120;
/// Absolute per-chapter note ceiling, regardless of density or chapter size.
pub const CHAPTER_NOTE_ABS_MAX: usize = 12;

/// Sink for a freshly annotated batch: `(block_index, source, note)` triples,
/// a skip note meaning "deliberately not annotated". The orchestrator persists
/// every triple to the cache and streams the non-skip ones to the UI.
pub type NoteSink<'a> = &'a mut (dyn FnMut(Vec<(usize, String, Note)>) + Send);

/// The code-enforced per-chapter note cap (AN-013): density scales how often a
/// chapter of `n` annotatable segments may stop, and nothing — model included —
/// can exceed it. This is what makes sparsity an architectural guarantee.
pub fn chapter_note_cap(density: Density, n: usize) -> usize {
    let base = match density {
        Density::Sparse => (n / 24).max(1),
        Density::Medium => (n / 12).max(1),
        Density::Rich => (n / 6).max(2),
    };
    base.min(CHAPTER_NOTE_ABS_MAX)
}

// ── Pass N0: plan ───────────────────────────────────────────────────────────

/// Book-wide annotation plan: theme map + reader-fit depth positioning + the
/// thread map (跨章線索圖) that gives the per-chapter selection pass forward
/// visibility — so a note can be PLACED after an insight has appeared and
/// reach BACK to already-read chapters, instead of discovering the connection
/// too late (or never). Threads inform placement only; note text never
/// previews unread content (hard rule 7).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnnotationPlan {
    #[serde(default)]
    pub themes: Vec<String>,
    #[serde(default)]
    pub focus: String,
    #[serde(default)]
    pub skip: String,
    #[serde(default)]
    pub guidance: String,
    /// Cross-chapter threads: "concept/setup — first appears ch N → pays off
    /// ch M" lines the selection pass uses to plan backward references.
    #[serde(default)]
    pub threads: Vec<String>,
}

impl AnnotationPlan {
    /// Fallback plan (mock provider, or a failed/unparseable plan call).
    pub fn fallback() -> Self {
        AnnotationPlan {
            themes: Vec::new(),
            focus: String::new(),
            skip: String::new(),
            guidance: "在概念首次出現、歷史事件、專門術語、與作者處境相關的段落停留。".into(),
            threads: Vec::new(),
        }
    }

    fn as_prompt_block(&self) -> String {
        let themes = if self.themes.is_empty() {
            "（無）".to_string()
        } else {
            self.themes.join("、")
        };
        let threads = if self.threads.is_empty() {
            "（無）".to_string()
        } else {
            self.threads
                .iter()
                .map(|t| format!("- {t}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        format!(
            "主題地圖：{themes}\n深入：{focus}\n略過：{skip}\n取材指引：{guidance}\n線索圖（後文視野，僅用於安排回指；眉批內文絕不預告後文）：\n{threads}",
            themes = themes,
            threads = threads,
            focus = if self.focus.is_empty() {
                "（依讀者背景自行判斷）"
            } else {
                &self.focus
            },
            skip = if self.skip.is_empty() {
                "（無）"
            } else {
                &self.skip
            },
            guidance = if self.guidance.is_empty() {
                "（依讀者背景自行判斷）"
            } else {
                &self.guidance
            },
        )
    }
}

/// Rolling memory across chapters: topics already annotated (the duplication
/// guard) + a program-built digest of the book so far. Serialized to job meta
/// after every chapter so resume keeps the guard.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnnoMemory {
    #[serde(default)]
    pub noted_topics: Vec<String>,
    #[serde(default)]
    pub digest: String,
}

impl AnnoMemory {
    fn topics_block(&self) -> String {
        if self.noted_topics.is_empty() {
            "（尚無）".to_string()
        } else {
            self.noted_topics.join("、")
        }
    }

    fn digest_block(&self) -> String {
        if self.digest.trim().is_empty() {
            "（本章為全書開頭）".to_string()
        } else {
            self.digest.clone()
        }
    }

    fn add_topics(&mut self, topics: impl IntoIterator<Item = String>) {
        for t in topics {
            let t = t.trim().to_string();
            if !t.is_empty() && !self.noted_topics.contains(&t) {
                self.noted_topics.push(t);
            }
        }
        if self.noted_topics.len() > TOPICS_CAP {
            let excess = self.noted_topics.len() - TOPICS_CAP;
            self.noted_topics.drain(..excess);
        }
    }

    /// Program-side rolling digest: deterministic and token-free (the LLM-grade
    /// duplication guard is `noted_topics`; the digest only anchors position).
    fn push_chapter_digest(&mut self, chapter: &Chapter) {
        let text = chapter_plain_text(chapter, 240);
        let line = format!("「{}」：{}", chapter.href, text.trim());
        if !self.digest.is_empty() {
            self.digest.push('\n');
        }
        self.digest.push_str(&line);
        let n = self.digest.chars().count();
        if n > DIGEST_CAP_CHARS {
            self.digest = self.digest.chars().skip(n - DIGEST_CAP_CHARS).collect();
        }
    }
}

fn chapter_plain_text(chapter: &Chapter, cap: usize) -> String {
    let mut s = String::new();
    for seg in &chapter.segments {
        s.push_str(&placeholder::strip_tokens(&seg.source));
        s.push('\n');
        if s.chars().count() >= cap {
            break;
        }
    }
    s.chars().take(cap).collect()
}

/// Head + mid sample of one chapter for the plan pass. The mid slice starts at
/// the chapter's halfway point; when the chapter is short enough that the head
/// already covers it, no mid slice is added (never duplicates text).
fn chapter_plan_sample(chapter: &Chapter, head: usize, mid: usize) -> String {
    let full: String = chapter
        .segments
        .iter()
        .map(|seg| {
            let mut t = placeholder::strip_tokens(&seg.source);
            t.push('\n');
            t
        })
        .collect();
    let chars: Vec<char> = full.chars().collect();
    let head_text: String = chars.iter().take(head).collect();
    if chars.len() <= head {
        return head_text;
    }
    let mid_start = (chars.len() / 2).max(head);
    let mid_text: String = chars[mid_start..(mid_start + mid).min(chars.len())]
        .iter()
        .collect();
    if mid_text.trim().is_empty() {
        return head_text;
    }
    format!("{head_text}\n…（章中段抽樣）…\n{mid_text}")
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

/// Pass N0: sample every chapter's opening (whole-book capped) and ask for the
/// annotation plan. Mock (and any failure) falls back to the default plan —
/// planning is an optimiser, never a blocker.
pub async fn plan(
    provider: &Provider,
    cfg: &TranslateConfig,
    anno: &AnnotationConfig,
    book: &Book,
) -> Result<(AnnotationPlan, Stats)> {
    let mut stats = Stats::default();
    if cfg.provider == ProviderKind::Mock {
        return Ok((AnnotationPlan::fallback(), stats));
    }

    let mut sample = String::new();
    for (ci, chapter) in book.chapters.iter().enumerate() {
        // Apparatus is not part of the book for planning or review purposes.
        if chapter.apparatus {
            continue;
        }
        let text = chapter_plan_sample(chapter, PLAN_CHARS_PER_CHAPTER, PLAN_MID_CHARS);
        if text.trim().is_empty() {
            continue;
        }
        sample.push_str(&format!(
            "【第 {} 章 {}】\n{}\n\n",
            ci + 1,
            chapter.href,
            text
        ));
        if sample.chars().count() >= PLAN_TOTAL_CHARS {
            break;
        }
    }

    let req = CompletionRequest {
        system: prompt::plan_system(anno),
        messages: vec![ChatMessage::user(sample)],
        temperature: 0.2,
    };
    let plan = match provider.complete_retrying(&req, 2).await {
        Ok(resp) => {
            stats.tokens_in += resp.tokens_in;
            stats.tokens_out += resp.tokens_out;
            extract_json_object(&resp.text)
                .and_then(|j| serde_json::from_str::<AnnotationPlan>(&j).ok())
                .unwrap_or_else(AnnotationPlan::fallback)
        }
        Err(_) => AnnotationPlan::fallback(),
    };
    Ok((plan, stats))
}

// ── Pass N-sel: per-chapter selection ───────────────────────────────────────

/// One selection-pass pick: where to stop, how to place the note, what angle
/// to take, and how important it is (the cap trims by priority).
#[derive(Debug, Clone)]
pub struct Selection {
    pub id: u64,
    pub pos: NotePos,
    pub angle: String,
    pub priority: i64,
}

#[derive(Deserialize)]
struct SelectionItem {
    id: u64,
    #[serde(default)]
    pos: Option<String>,
    #[serde(default)]
    angle: String,
    #[serde(default)]
    priority: Option<i64>,
}

#[derive(Deserialize, Default)]
struct SelectionsResponse {
    #[serde(default)]
    selections: Vec<SelectionItem>,
}

/// Parse + program-validate one selection window response: every id must have
/// been sent, `pos` must be legal, duplicate picks collapse (first wins), and
/// a missing priority ranks lowest.
fn parse_selections_response(raw: &str, ids: &HashSet<u64>) -> Result<Vec<Selection>> {
    let json = extract_json_object(raw).ok_or_else(|| {
        crate::error::CoreError::Validation("selection response has no JSON object".into())
    })?;
    let parsed: SelectionsResponse = serde_json::from_str(&json).map_err(|e| {
        crate::error::CoreError::Validation(format!("selection response malformed: {e}"))
    })?;
    let mut seen: HashSet<u64> = HashSet::new();
    let mut out = Vec::new();
    for item in parsed.selections {
        if !ids.contains(&item.id) {
            return Err(crate::error::CoreError::Validation(format!(
                "selection for unknown id {}",
                item.id
            )));
        }
        if !seen.insert(item.id) {
            continue; // duplicate pick — the first one wins
        }
        let pos = match item.pos.as_deref() {
            None | Some("after") => NotePos::After,
            Some("before") => NotePos::Before,
            Some(other) => {
                return Err(crate::error::CoreError::Validation(format!(
                    "illegal note position {other:?} for id {}",
                    item.id
                )))
            }
        };
        out.push(Selection {
            id: item.id,
            pos,
            angle: item.angle.trim().to_string(),
            priority: item.priority.unwrap_or(i64::MIN),
        });
    }
    Ok(out)
}

/// Enforce the chapter cap (AN-013): keep the `cap` highest-priority picks
/// (ties broken by document order), returned in document order. The model can
/// ask for anything; this function is why it can't get more.
fn enforce_cap(
    mut sels: Vec<Selection>,
    cap: usize,
    doc_order: &HashMap<u64, usize>,
) -> Vec<Selection> {
    let order = |id: &u64| doc_order.get(id).copied().unwrap_or(usize::MAX);
    if sels.len() > cap {
        sels.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| order(&a.id).cmp(&order(&b.id)))
        });
        sels.truncate(cap);
    }
    sels.sort_by_key(|s| order(&s.id));
    sels
}

fn snippet(text: &str, cap: usize) -> String {
    text.chars().take(cap).collect()
}

/// Split `n` snippet lengths into contiguous windows within a char budget
/// (always at least one unit per window). Pure so the split is testable.
fn window_spans(lens: &[usize], budget: usize) -> Vec<std::ops::Range<usize>> {
    let mut spans = Vec::new();
    let mut start = 0usize;
    let mut acc = 0usize;
    for (i, l) in lens.iter().enumerate() {
        if i > start && acc + l > budget {
            spans.push(start..i);
            start = i;
            acc = 0;
        }
        acc += l;
    }
    if start < lens.len() {
        spans.push(start..lens.len());
    }
    spans
}

/// A window's share of the chapter cap, proportional to its unit count
/// (rounded up, at least 1). The global cap is enforced again after all
/// windows, so per-window generosity can never leak past `enforce_cap`.
fn window_quota(cap: usize, window_units: usize, total_units: usize) -> usize {
    if total_units == 0 {
        return 0;
    }
    ((cap * window_units).div_ceil(total_units)).max(1)
}

/// Selection outcome for one chapter: the capped picks, plus which pending ids
/// were resolved by a successful window (a resolved-but-unpicked id becomes the
/// cached "deliberately skipped"; ids of failed windows stay undecided so the
/// next run retries them).
struct SelectOutcome {
    selections: Vec<Selection>,
    resolved: HashSet<u64>,
    stats: Stats,
}

/// Pass N-sel for one chapter: the whole chapter compressed into head snippets
/// (windowed when huge, later windows seeing earlier picks read-only) goes to
/// the model, which returns `{id, pos, angle, priority}` picks — no note text.
/// The result is trimmed to the program-side cap before any note is written.
async fn select_chapter(
    provider: &Provider,
    pending: &[(u64, String)],
    cap: usize,
    system: &str,
) -> SelectOutcome {
    let mut stats = Stats::default();
    let snippets: Vec<(u64, String)> = pending
        .iter()
        .map(|(id, text)| (*id, snippet(text, SELECT_SNIPPET_CHARS)))
        .collect();
    let lens: Vec<usize> = snippets.iter().map(|(_, s)| s.chars().count()).collect();
    let doc_order: HashMap<u64, usize> = pending
        .iter()
        .enumerate()
        .map(|(i, (id, _))| (*id, i))
        .collect();

    let mut all: Vec<Selection> = Vec::new();
    let mut resolved: HashSet<u64> = HashSet::new();
    for span in window_spans(&lens, SELECT_WINDOW_CHARS) {
        let window = &snippets[span];
        let quota = window_quota(cap, window.len(), snippets.len());
        let units: Vec<_> = window
            .iter()
            .map(|(id, text)| json!({ "id": id, "text": text }))
            .collect();
        let already: Vec<_> = all
            .iter()
            .map(|s| json!({ "id": s.id, "angle": s.angle }))
            .collect();
        let payload = json!({
            "units": units,
            "max_selections": quota,
            "already_selected": already,
        })
        .to_string();
        let req = CompletionRequest {
            system: system.to_string(),
            messages: vec![ChatMessage::user(payload)],
            temperature: 0.2,
        };
        let ids: HashSet<u64> = window.iter().map(|(id, _)| *id).collect();
        // A failed window (call or validation) leaves its segments UNDECIDED —
        // nothing cached — so the next run retries them; it never fakes skips.
        match provider.complete_retrying(&req, 2).await {
            Ok(resp) => {
                stats.tokens_in += resp.tokens_in;
                stats.tokens_out += resp.tokens_out;
                match parse_selections_response(&resp.text, &ids) {
                    Ok(sels) => {
                        resolved.extend(ids);
                        all.extend(sels);
                    }
                    Err(_) => stats.units_failed += 1,
                }
            }
            Err(e) => {
                stats.units_failed += 1;
                stats.sample_error.get_or_insert_with(|| e.to_string());
            }
        }
    }
    SelectOutcome {
        selections: enforce_cap(all, cap, &doc_order),
        resolved,
        stats,
    }
}

// ── Pass N1: notes for the selected segments ────────────────────────────────

#[derive(Deserialize, Default)]
struct NoteItem {
    id: u64,
    #[serde(default)]
    note: String,
}

#[derive(Deserialize, Default)]
struct NotesResponse {
    #[serde(default)]
    notes: Vec<NoteItem>,
    #[serde(default)]
    topics: Vec<String>,
}

/// A note may not carry protocol placeholders (it is NEW text, never aligned
/// against a source) — a `⟦` in a note would leak raw tokens into the output.
/// It may also never address or characterise the reader (the Fable rule,
/// machine-enforced): the reader profile steers selection and bridging only.
fn note_text_ok(note: &str) -> bool {
    !note.trim().is_empty()
        && !note.contains(placeholder::OPEN)
        && !note.contains(placeholder::CLOSE)
        && !prompt::note_addresses_reader(note)
        && !prompt::note_opens_generic(note)
}

/// Parse + program-validate one N1 batch response: every returned id must have
/// been sent, no duplicates, note text non-empty and placeholder-free.
fn parse_notes_response(
    raw: &str,
    ids: &HashSet<u64>,
) -> Result<(HashMap<u64, String>, Vec<String>)> {
    let json = extract_json_object(raw).ok_or_else(|| {
        crate::error::CoreError::Validation("annotation response has no JSON object".into())
    })?;
    let parsed: NotesResponse = serde_json::from_str(&json).map_err(|e| {
        crate::error::CoreError::Validation(format!("annotation response malformed: {e}"))
    })?;
    let mut out = HashMap::new();
    for item in parsed.notes {
        if !ids.contains(&item.id) {
            return Err(crate::error::CoreError::Validation(format!(
                "annotation for unknown id {}",
                item.id
            )));
        }
        if out.contains_key(&item.id) {
            return Err(crate::error::CoreError::Validation(format!(
                "duplicate annotation id {}",
                item.id
            )));
        }
        if !note_text_ok(&item.note) {
            return Err(crate::error::CoreError::Validation(format!(
                "empty or placeholder-carrying note for id {}",
                item.id
            )));
        }
        out.insert(item.id, item.note.trim().to_string());
    }
    Ok((out, parsed.topics))
}

/// One annotated batch: id → note for noted units, the ids that were resolved
/// at all (a resolved-but-unnoted id is the cached "deliberately skipped"),
/// new topics, and the stats accrued.
struct NoteOutcome {
    notes: HashMap<u64, String>,
    resolved: Vec<u64>,
    topics: Vec<String>,
    stats: Stats,
}

/// Annotate one batch with the same split-on-failure fallback the translator
/// uses. Each unit is a pre-built JSON object (id, text, angle, pos, context).
/// A single-unit failure leaves the unit UNRESOLVED (note stays `None`,
/// nothing cached) so the next run retries it.
fn annotate_batch<'a>(
    provider: &'a Provider,
    batch: &'a [(u64, serde_json::Value)],
    system: &'a str,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = NoteOutcome> + Send + 'a>> {
    Box::pin(async move {
        let mut stats = Stats::default();
        match annotate_once(provider, batch, system, &mut stats).await {
            Ok((notes, topics)) => NoteOutcome {
                resolved: batch.iter().map(|(id, _)| *id).collect(),
                notes,
                topics,
                stats,
            },
            Err(_) if batch.len() > 1 => {
                let mid = batch.len() / 2;
                let a = annotate_batch(provider, &batch[..mid], system).await;
                let b = annotate_batch(provider, &batch[mid..], system).await;
                let mut notes = a.notes;
                notes.extend(b.notes);
                let mut resolved = a.resolved;
                resolved.extend(b.resolved);
                let mut topics = a.topics;
                topics.extend(b.topics);
                stats.merge(&a.stats);
                stats.merge(&b.stats);
                NoteOutcome {
                    notes,
                    resolved,
                    topics,
                    stats,
                }
            }
            Err(e) => {
                stats.units_failed += 1;
                stats.sample_error.get_or_insert_with(|| e.to_string());
                NoteOutcome {
                    notes: HashMap::new(),
                    resolved: Vec::new(),
                    topics: Vec::new(),
                    stats,
                }
            }
        }
    })
}

async fn annotate_once(
    provider: &Provider,
    batch: &[(u64, serde_json::Value)],
    system: &str,
    stats: &mut Stats,
) -> Result<(HashMap<u64, String>, Vec<String>)> {
    let units: Vec<_> = batch.iter().map(|(_, unit)| unit.clone()).collect();
    let payload = json!({ "units": units }).to_string();
    let req = CompletionRequest {
        system: system.to_string(),
        messages: vec![ChatMessage::user(payload)],
        temperature: 0.3,
    };
    let resp = provider.complete_retrying(&req, 2).await?;
    stats.tokens_in += resp.tokens_in;
    stats.tokens_out += resp.tokens_out;
    let ids: HashSet<u64> = batch.iter().map(|(id, _)| *id).collect();
    parse_notes_response(&resp.text, &ids)
}

/// Passes N-sel + N1 for one chapter: select WHERE to stop (whole chapter
/// compressed, program-side hard cap), mark everything unpicked "deliberately
/// skipped", then write notes only for the selected segments (each carrying
/// its angle, placement and neighbouring context). Rolls the memory forward.
/// Batches are sequential (the rolling memory is order-dependent and the
/// annotation volume is a fraction of translation's).
pub async fn annotate_chapter(
    provider: &Provider,
    cfg: &TranslateConfig,
    anno: &AnnotationConfig,
    plan: &AnnotationPlan,
    mem: &mut AnnoMemory,
    chapter: &mut Chapter,
    on_batch: NoteSink<'_>,
) -> Result<Stats> {
    let mut stats = Stats::default();

    // Segments with no visible text can never carry a note: resolve them as
    // "deliberately skipped" up front (cached, zero tokens).
    let silent: Vec<(usize, String, Note)> = chapter
        .segments
        .iter_mut()
        .filter(|s| s.note.is_none() && !has_visible_text(&s.source))
        .map(|s| {
            s.note = Some(Note::skip());
            (s.block_index, s.source.clone(), Note::skip())
        })
        .collect();
    if !silent.is_empty() {
        on_batch(silent);
    }

    let pending: Vec<(u64, String)> = chapter
        .segments
        .iter()
        .filter(|s| s.note.is_none())
        .map(|s| (s.block_index as u64, placeholder::strip_tokens(&s.source)))
        .collect();
    if pending.is_empty() {
        return Ok(stats);
    }

    // AN-013 hard cap: density × annotatable-segment count, minus what earlier
    // (cached) runs already wrote in this chapter. Zero remaining budget means
    // everything still pending is a guaranteed skip — no LLM call can add more.
    let visible_n = chapter
        .segments
        .iter()
        .filter(|s| has_visible_text(&s.source))
        .count();
    let already_noted = chapter
        .segments
        .iter()
        .filter(|s| s.note.as_ref().is_some_and(|n| !n.is_skip()))
        .count();
    let cap = chapter_note_cap(anno.density, visible_n).saturating_sub(already_noted);
    if cap == 0 {
        let mut skips: Vec<(usize, String, Note)> = Vec::new();
        for seg in &mut chapter.segments {
            if seg.note.is_none() {
                seg.note = Some(Note::skip());
                skips.push((seg.block_index, seg.source.clone(), Note::skip()));
            }
        }
        if !skips.is_empty() {
            on_batch(skips);
        }
        mem.push_chapter_digest(chapter);
        return Ok(stats);
    }

    // Pass N-sel: pick the stopping points (capped program-side).
    let select_sys = prompt::select_system(
        anno,
        &plan.as_prompt_block(),
        &mem.topics_block(),
        &mem.digest_block(),
    );
    let sel = select_chapter(provider, &pending, cap, &select_sys).await;
    stats.merge(&sel.stats);

    // Everything a successful window resolved but selection didn't pick is a
    // deliberate skip — cached so resume never re-asks.
    let picked_ids: HashSet<u64> = sel.selections.iter().map(|s| s.id).collect();
    let mut skips: Vec<(usize, String, Note)> = Vec::new();
    for seg in &mut chapter.segments {
        let id = seg.block_index as u64;
        if seg.note.is_some() || !sel.resolved.contains(&id) || picked_ids.contains(&id) {
            continue;
        }
        seg.note = Some(Note::skip());
        skips.push((seg.block_index, seg.source.clone(), Note::skip()));
    }
    if !skips.is_empty() {
        on_batch(skips);
    }
    if sel.selections.is_empty() {
        mem.push_chapter_digest(chapter);
        return Ok(stats);
    }

    // Pass N1: write a note for each selected segment, carrying the selection's
    // angle + placement and the nearest visible neighbours as context.
    let stripped: Vec<String> = chapter
        .segments
        .iter()
        .map(|s| placeholder::strip_tokens(&s.source))
        .collect();
    let visible: Vec<bool> = chapter
        .segments
        .iter()
        .map(|s| has_visible_text(&s.source))
        .collect();
    let idx_of: HashMap<u64, usize> = chapter
        .segments
        .iter()
        .enumerate()
        .map(|(i, s)| (s.block_index as u64, i))
        .collect();
    let pending_map: HashMap<u64, String> = pending.into_iter().collect();
    let ctx_before = |si: usize| -> String {
        (0..si)
            .rev()
            .find(|&j| visible[j])
            .map(|j| snippet(&stripped[j], GEN_CONTEXT_CHARS))
            .unwrap_or_default()
    };
    let ctx_after = |si: usize| -> String {
        (si + 1..stripped.len())
            .find(|&j| visible[j])
            .map(|j| snippet(&stripped[j], GEN_CONTEXT_CHARS))
            .unwrap_or_default()
    };
    let mut units: Vec<(u64, serde_json::Value)> = Vec::new();
    for s in &sel.selections {
        let (Some(text), Some(&si)) = (pending_map.get(&s.id), idx_of.get(&s.id)) else {
            continue;
        };
        units.push((
            s.id,
            json!({
                "id": s.id,
                "text": text,
                "angle": s.angle,
                "pos": s.pos,
                "context_before": ctx_before(si),
                "context_after": ctx_after(si),
            }),
        ));
    }
    let pos_of: HashMap<u64, NotePos> = sel.selections.iter().map(|s| (s.id, s.pos)).collect();

    let system = prompt::notes_system(
        anno,
        &plan.as_prompt_block(),
        &mem.topics_block(),
        &mem.digest_block(),
    );

    // Batch by the translator's caps.
    let mut work: Vec<Vec<(u64, serde_json::Value)>> = Vec::new();
    let mut cur: Vec<(u64, serde_json::Value)> = Vec::new();
    let mut cur_tokens = 0usize;
    for (id, unit) in units {
        let chars = unit["text"]
            .as_str()
            .map(|t| t.chars().count())
            .unwrap_or(0);
        let t = (chars as f32 / 2.5).ceil() as usize;
        if !cur.is_empty()
            && (cur.len() >= cfg.max_batch_sentences || cur_tokens + t > cfg.max_chunk_tokens)
        {
            work.push(std::mem::take(&mut cur));
            cur_tokens = 0;
        }
        cur.push((id, unit));
        cur_tokens += t;
    }
    if !cur.is_empty() {
        work.push(cur);
    }

    for batch in work {
        let outcome = annotate_batch(provider, &batch, &system).await;
        stats.merge(&outcome.stats);
        let resolved: HashSet<u64> = outcome.resolved.iter().copied().collect();
        let mut sink: Vec<(usize, String, Note)> = Vec::new();
        for seg in &mut chapter.segments {
            let id = seg.block_index as u64;
            if seg.note.is_some() || !resolved.contains(&id) {
                continue;
            }
            // The placement comes from the selection; the model only wrote the
            // text. A selected unit the model declined becomes a cached skip.
            let note = match outcome.notes.get(&id) {
                Some(text) => {
                    stats.units_translated += 1;
                    Note::new(pos_of.get(&id).copied().unwrap_or_default(), text.clone())
                }
                None => Note::skip(),
            };
            seg.note = Some(note.clone());
            sink.push((seg.block_index, seg.source.clone(), note));
        }
        if !sink.is_empty() {
            on_batch(sink);
        }
        mem.add_topics(outcome.topics);
    }

    mem.push_chapter_digest(chapter);
    Ok(stats)
}

// ── Pass N2: book-wide review ───────────────────────────────────────────────

/// Observable evidence of the review pass (surfaces in RunSummary / CLI JSON).
#[derive(Debug, Clone, Copy, Default)]
pub struct ReviewOutcome {
    pub kept: usize,
    pub edited: usize,
    pub dropped: usize,
}

#[derive(Deserialize)]
struct Verdict {
    id: u64,
    action: String,
    #[serde(default)]
    note: Option<String>,
}

fn parse_review_response(
    raw: &str,
    ids: &HashSet<u64>,
) -> Result<HashMap<u64, (String, Option<String>)>> {
    // Reuse the array extractor from validate.rs semantics: find the array.
    let start = raw.find('[').ok_or_else(|| {
        crate::error::CoreError::Validation("review response has no JSON array".into())
    })?;
    let end = raw.rfind(']').ok_or_else(|| {
        crate::error::CoreError::Validation("review response has no JSON array".into())
    })?;
    let verdicts: Vec<Verdict> = serde_json::from_str(&raw[start..=end])
        .map_err(|e| crate::error::CoreError::Validation(format!("review malformed: {e}")))?;
    let mut out = HashMap::new();
    for v in verdicts {
        if !ids.contains(&v.id) {
            return Err(crate::error::CoreError::Validation(format!(
                "review verdict for unknown id {}",
                v.id
            )));
        }
        match v.action.as_str() {
            "keep" | "drop" => {}
            "edit" => {
                let ok = v.note.as_deref().is_some_and(note_text_ok);
                if !ok {
                    return Err(crate::error::CoreError::Validation(format!(
                        "edit verdict without a valid note for id {}",
                        v.id
                    )));
                }
            }
            other => {
                return Err(crate::error::CoreError::Validation(format!(
                    "unknown review action {other:?}"
                )))
            }
        }
        out.insert(v.id, (v.action, v.note));
    }
    Ok(out)
}

/// Pass N2: unify the whole book's notes. Program-side exact-duplicate removal
/// first, then batched model review (keep/edit/drop). Mutates `book`; the
/// orchestrator re-persists the affected cache entries. Structurally incapable
/// of inventing notes for segments that have none (AN-011): it only rewrites
/// the collected references.
pub async fn review(
    provider: &Provider,
    anno: &AnnotationConfig,
    book: &mut Book,
) -> Result<(ReviewOutcome, Stats)> {
    let mut stats = Stats::default();
    let mut outcome = ReviewOutcome::default();

    // Collect every non-skip note in reading order.
    let mut refs: Vec<(usize, usize)> = Vec::new(); // (chapter idx, segment idx)
    for (ci, chapter) in book.chapters.iter().enumerate() {
        // Apparatus is not part of the book for planning or review purposes.
        if chapter.apparatus {
            continue;
        }
        for (si, seg) in chapter.segments.iter().enumerate() {
            if seg.note.as_ref().is_some_and(|n| !n.is_skip()) {
                refs.push((ci, si));
            }
        }
    }

    // Program-side pass: exact duplicates never reach the model.
    let mut seen: HashSet<String> = HashSet::new();
    let mut entries: Vec<(u64, usize, usize)> = Vec::new(); // (id, ci, si)
    for (ci, si) in refs {
        let note = book.chapters[ci].segments[si]
            .note
            .clone()
            .unwrap_or_else(Note::skip);
        let norm = note.text.trim().to_string();
        if seen.contains(&norm) {
            book.chapters[ci].segments[si].note = Some(Note::skip());
            outcome.dropped += 1;
            continue;
        }
        seen.insert(norm);
        let id = entries.len() as u64;
        entries.push((id, ci, si));
    }
    if entries.is_empty() {
        return Ok((outcome, stats));
    }

    let system = prompt::review_system(anno);
    let note_of = |book: &Book, ci: usize, si: usize| -> Note {
        book.chapters[ci].segments[si]
            .note
            .clone()
            .unwrap_or_else(Note::skip)
    };

    for window in entries.chunks(REVIEW_BATCH) {
        let judge: Vec<_> = window
            .iter()
            .map(|(id, ci, si)| {
                let n = note_of(book, *ci, *si);
                json!({ "id": id, "chapter": ci, "pos": n.pos, "note": n.text })
            })
            .collect();
        let window_ids: HashSet<u64> = window.iter().map(|(id, _, _)| *id).collect();
        let others: Vec<_> = entries
            .iter()
            .filter(|(id, _, _)| !window_ids.contains(id))
            .map(|(id, ci, si)| {
                let n: String = note_of(book, *ci, *si)
                    .text
                    .chars()
                    .take(REVIEW_CONTEXT_CHARS)
                    .collect();
                json!({ "id": id, "note": n })
            })
            .collect();
        let payload = json!({
            "reader_profile": anno.reader_profile,
            "judge": judge,
            "others": others,
        })
        .to_string();
        let req = CompletionRequest {
            system: system.clone(),
            messages: vec![ChatMessage::user(payload)],
            temperature: 0.2,
        };
        let verdicts = match provider.complete_retrying(&req, 2).await {
            Ok(resp) => {
                stats.tokens_in += resp.tokens_in;
                stats.tokens_out += resp.tokens_out;
                parse_review_response(&resp.text, &window_ids).unwrap_or_default()
            }
            // Fail-open: a broken review call must never destroy written notes.
            Err(_) => HashMap::new(),
        };
        for (id, ci, si) in window {
            match verdicts.get(id) {
                Some((action, note)) => match action.as_str() {
                    "drop" => {
                        book.chapters[*ci].segments[*si].note = Some(Note::skip());
                        outcome.dropped += 1;
                    }
                    "edit" => {
                        // AN-014: placement is the selection pass's decision —
                        // an edit rewrites the text but can never move the note.
                        let n = note.clone().unwrap_or_default().trim().to_string();
                        let pos = note_of(book, *ci, *si).pos;
                        book.chapters[*ci].segments[*si].note = Some(Note::new(pos, n));
                        outcome.edited += 1;
                    }
                    _ => outcome.kept += 1,
                },
                None => outcome.kept += 1, // unmentioned = keep
            }
        }
    }
    Ok((outcome, stats))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_response_validation() {
        let ids: HashSet<u64> = [1, 2, 3].into_iter().collect();
        // sparse + skipping is legal
        let (notes, topics) = parse_notes_response(
            r#"{"notes":[{"id":2,"note":"背景"}],"topics":["史"]}"#,
            &ids,
        )
        .unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(topics, vec!["史"]);
        // empty is legal (whole batch skipped)
        assert!(parse_notes_response(r#"{"notes":[],"topics":[]}"#, &ids)
            .unwrap()
            .0
            .is_empty());
        // unknown id rejected
        assert!(parse_notes_response(r#"{"notes":[{"id":9,"note":"x"}]}"#, &ids).is_err());
        // empty note rejected
        assert!(parse_notes_response(r#"{"notes":[{"id":1,"note":"  "}]}"#, &ids).is_err());
        // placeholder-carrying note rejected
        assert!(parse_notes_response(r#"{"notes":[{"id":1,"note":"壞⟦1⟧"}]}"#, &ids).is_err());
        // duplicate id rejected
        assert!(parse_notes_response(
            r#"{"notes":[{"id":1,"note":"a"},{"id":1,"note":"b"}]}"#,
            &ids
        )
        .is_err());
    }

    #[test]
    fn selections_response_validation() {
        let ids: HashSet<u64> = [1, 2, 3].into_iter().collect();
        // legal picks, pos + priority parsed
        let sels = parse_selections_response(
            r#"{"selections":[{"id":2,"pos":"before","angle":"史","priority":7},{"id":3,"pos":"after","angle":"術語"}]}"#,
            &ids,
        )
        .unwrap();
        assert_eq!(sels.len(), 2);
        assert_eq!(sels[0].pos, NotePos::Before);
        assert_eq!(sels[0].priority, 7);
        // missing priority ranks lowest
        assert_eq!(sels[1].priority, i64::MIN);
        // missing pos defaults to after
        let sels =
            parse_selections_response(r#"{"selections":[{"id":1,"angle":"x"}]}"#, &ids).unwrap();
        assert_eq!(sels[0].pos, NotePos::After);
        // empty is legal
        assert!(parse_selections_response(r#"{"selections":[]}"#, &ids)
            .unwrap()
            .is_empty());
        // unknown id rejected
        assert!(
            parse_selections_response(r#"{"selections":[{"id":9,"angle":"x"}]}"#, &ids).is_err()
        );
        // illegal pos rejected
        assert!(parse_selections_response(
            r#"{"selections":[{"id":1,"pos":"inside","angle":"x"}]}"#,
            &ids
        )
        .is_err());
        // duplicate pick: first wins, not an error
        let sels = parse_selections_response(
            r#"{"selections":[{"id":1,"pos":"before","angle":"a","priority":2},{"id":1,"pos":"after","angle":"b","priority":9}]}"#,
            &ids,
        )
        .unwrap();
        assert_eq!(sels.len(), 1);
        assert_eq!(sels[0].pos, NotePos::Before);
    }

    #[test]
    fn cap_trims_by_priority_then_document_order() {
        let doc_order: HashMap<u64, usize> = (0..5u64).map(|i| (i, i as usize)).collect();
        let sel = |id: u64, priority: i64| Selection {
            id,
            pos: NotePos::After,
            angle: String::new(),
            priority,
        };
        // over cap: highest priority survives, output back in document order
        let sels = vec![sel(0, 1), sel(1, 9), sel(2, 5), sel(3, 9), sel(4, i64::MIN)];
        let kept = enforce_cap(sels, 3, &doc_order);
        assert_eq!(kept.iter().map(|s| s.id).collect::<Vec<_>>(), vec![1, 2, 3]);
        // priority tie broken by document order
        let sels = vec![sel(3, 5), sel(1, 5), sel(2, 5)];
        let kept = enforce_cap(sels, 2, &doc_order);
        assert_eq!(kept.iter().map(|s| s.id).collect::<Vec<_>>(), vec![1, 2]);
        // under cap: untouched (just ordered)
        let sels = vec![sel(2, 1), sel(0, 1)];
        let kept = enforce_cap(sels, 5, &doc_order);
        assert_eq!(kept.iter().map(|s| s.id).collect::<Vec<_>>(), vec![0, 2]);
    }

    #[test]
    fn chapter_note_cap_formula() {
        use Density::*;
        // 精 ≈ n/24, floor 1
        assert_eq!(chapter_note_cap(Sparse, 5), 1);
        assert_eq!(chapter_note_cap(Sparse, 48), 2);
        // 適中 ≈ n/12, floor 1
        assert_eq!(chapter_note_cap(Medium, 5), 1);
        assert_eq!(chapter_note_cap(Medium, 30), 2);
        assert_eq!(chapter_note_cap(Medium, 120), 10);
        // 豐 ≈ n/6, floor 2
        assert_eq!(chapter_note_cap(Rich, 5), 2);
        assert_eq!(chapter_note_cap(Rich, 30), 5);
        // absolute ceiling regardless of size
        assert_eq!(chapter_note_cap(Rich, 10_000), CHAPTER_NOTE_ABS_MAX);
        assert_eq!(chapter_note_cap(Medium, 10_000), CHAPTER_NOTE_ABS_MAX);
    }

    #[test]
    fn selection_windows_and_quota() {
        // 5 snippets of 100 chars under a 250-char budget → 3 windows (2/2/1)
        let lens = [100, 100, 100, 100, 100];
        let spans = window_spans(&lens, 250);
        assert_eq!(spans, vec![0..2, 2..4, 4..5]);
        // one unit larger than the budget still forms its own window
        let spans = window_spans(&[500, 100], 250);
        assert_eq!(spans, vec![0..1, 1..2]);
        // empty input → no windows
        assert!(window_spans(&[], 250).is_empty());
        // quota is proportional (ceil), at least 1
        assert_eq!(window_quota(6, 2, 5), 3);
        assert_eq!(window_quota(1, 2, 5), 1);
        assert_eq!(window_quota(0, 2, 5), 1); // still ≥1 per window; global cap trims after
        assert_eq!(window_quota(4, 5, 5), 4);
    }

    // The thread map reaches the prompts through the plan block (selection
    // reads it to place backward references), and an empty map degrades to a
    // harmless "（無）" line.
    #[test]
    fn plan_block_carries_thread_map() {
        let plan = AnnotationPlan {
            themes: vec!["分工".into()],
            focus: String::new(),
            skip: String::new(),
            guidance: String::new(),
            threads: vec!["分工：第1章針廠首現 → 第5卷「變鈍」收束".into()],
        };
        let block = plan.as_prompt_block();
        assert!(block.contains("線索圖"));
        assert!(block.contains("- 分工：第1章針廠首現 → 第5卷「變鈍」收束"));
        assert!(
            block.contains("絕不預告後文"),
            "anti-spoiler framing attached"
        );
        assert!(AnnotationPlan::fallback()
            .as_prompt_block()
            .contains("（無）"));
    }

    // The reader-boundary check guards BOTH places that accept note text:
    // the N1 writing response and the N2 edit verdict.
    #[test]
    fn reader_addressing_notes_are_rejected_at_both_chokepoints() {
        let ids: HashSet<u64> = [1].into_iter().collect();
        assert!(parse_notes_response(
            r#"{"notes":[{"id":1,"note":"就像你拆函式一樣，斯密拆了製針。"}]}"#,
            &ids,
        )
        .is_err());
        let ids: HashSet<u64> = [0].into_iter().collect();
        assert!(parse_review_response(
            r#"[{"id":0,"action":"edit","note":"身為讀者，你會看出這是伏筆。"}]"#,
            &ids,
        )
        .is_err());
        // A neutral note still passes both.
        let ids: HashSet<u64> = [1].into_iter().collect();
        assert!(parse_notes_response(
            r#"{"notes":[{"id":1,"note":"1931 年 Adams 才鑄出「美國夢」一詞。"}]}"#,
            &ids,
        )
        .is_ok());
    }

    #[test]
    fn review_response_validation() {
        let ids: HashSet<u64> = [0, 1].into_iter().collect();
        let ok = r#"[{"id":0,"action":"keep"},{"id":1,"action":"edit","note":"改"}]"#;
        let v = parse_review_response(ok, &ids).unwrap();
        assert_eq!(v[&1].0, "edit");
        // unknown id rejected
        assert!(parse_review_response(r#"[{"id":7,"action":"keep"}]"#, &ids).is_err());
        // edit without note rejected
        assert!(parse_review_response(r#"[{"id":0,"action":"edit"}]"#, &ids).is_err());
        // unknown action rejected
        assert!(parse_review_response(r#"[{"id":0,"action":"merge"}]"#, &ids).is_err());
    }
}
