//! Translation orchestration.
//!
//! - **Sentence** level: batched JSON in/out at block granularity with hard
//!   alignment validation and a three-tier fallback (whole batch → split → single).
//! - **Expert** level: multi-pass (glossary pre-scan → glossary/style/summary
//!   constrained draft → reflection → consistency), see [`expert`].
//!
//! Both flow through [`run`], which owns the book loop: cache prefill, optional
//! pre-scan, per-chapter translate + checkpoint, and (expert) a consistency pass.
//! CLI and desktop share it via a progress callback.

pub mod expert;
pub mod glossary_sub;
pub mod prompt;

/// Streaming sink for freshly translated (source, target) pairs.
/// One batch's streamed output.
pub struct BatchPairs {
    /// (source, target) for the live read-along. Includes units that fell back
    /// to their source, so a failed segment still fills its preview slot.
    pub pairs: Vec<(String, String)>,
    /// The subset safe to persist in the resume cache. A failed unit's
    /// source-fallback is deliberately absent: cached, it is indistinguishable
    /// from a real translation, so re-running the same command would report a
    /// clean success and hand back the original text forever.
    pub cacheable: Vec<(String, String)>,
}

pub type PairSink<'a> = &'a mut (dyn FnMut(BatchPairs) + Send);

/// Whether a segment's source carries visible text (placeholder-only segments
/// such as bare images/breaks are skipped by read-along UIs).
pub fn has_visible_text(src: &str) -> bool {
    let mut in_ph = false;
    src.chars().any(|c| match c {
        '⟦' => {
            in_ph = true;
            false
        }
        '⟧' => {
            in_ph = false;
            false
        }
        c => !in_ph && !c.is_whitespace(),
    })
}

use crate::config::{Level, TranslateConfig};
use crate::document::Book;
use crate::error::Result;
use crate::job::JobStore;
use crate::llm::{ChatMessage, CompletionRequest, Provider};
use crate::memory::ExpertMemory;
use crate::validate;
use serde_json::json;
use std::collections::HashMap;

#[derive(Debug, Default, Clone)]
pub struct Stats {
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub units_translated: usize,
    pub units_failed: usize,
    /// Unit ids (block indices) that fell back to their source. They still go
    /// into the assembled output — a book with an untranslated line beats no
    /// book — but they must never reach the resume cache: once cached, a
    /// failure is indistinguishable from a translation, so re-running the same
    /// command reports a clean success and returns the original text forever.
    pub failed_units: std::collections::HashSet<u64>,
    /// The first provider error seen, kept so a run where everything failed
    /// can say WHY instead of only counting. Provider errors are already
    /// credential-redacted at the llm layer.
    pub sample_error: Option<String>,
}

impl Stats {
    pub(crate) fn merge(&mut self, other: &Stats) {
        self.failed_units.extend(other.failed_units.iter().copied());
        self.tokens_in += other.tokens_in;
        self.tokens_out += other.tokens_out;
        self.units_translated += other.units_translated;
        self.units_failed += other.units_failed;
        if self.sample_error.is_none() {
            self.sample_error = other.sample_error.clone();
        }
    }
}

/// Which phase a progress callback is reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Prescan,
    Translating,
    Consistency,
    /// A translation batch just landed (mid-chapter). `pairs` carries its
    /// (source, target) pairs so UIs can stream a real-time read-along.
    Batch,
    /// Annotation pass N0: building the book-wide annotation plan.
    AnnotatePlan,
    /// Annotation pass N1. Batch events carry `notes`; the chapter-boundary
    /// event carries the chapter's note counters instead.
    Annotating,
    /// Annotation pass N2: the book-wide unification review.
    AnnotateReview,
}

#[derive(Debug, Clone)]
pub struct RunProgress {
    pub phase: Phase,
    pub chapter_index: usize,
    pub total_chapters: usize,
    pub href: String,
    pub sample: Option<String>,
    pub units_translated: usize,
    pub units_failed: usize,
    /// (source, target) pairs of the chapter this event reports (Translating
    /// only; capped). Lets UIs stream a live 原文/譯文 interleave.
    pub pairs: Vec<(String, String)>,
    /// (source, note) pairs of a freshly annotated batch (Annotating only;
    /// visible-text, non-skip notes). The note carries its placement (AN-014)
    /// so read-along UIs can render before-notes above the paragraph.
    pub notes: Vec<(String, crate::document::Note)>,
}

impl RunProgress {
    fn at(phase: Phase, chapter_index: usize, total_chapters: usize) -> Self {
        Self {
            phase,
            chapter_index,
            total_chapters,
            href: String::new(),
            sample: None,
            units_translated: 0,
            units_failed: 0,
            pairs: Vec::new(),
            notes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct RunSummary {
    pub restored_from_cache: usize,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub units_translated: usize,
    pub units_failed: usize,
    pub glossary_size: usize,
    pub inconsistencies: Vec<String>,
    /// Annotation evidence: non-empty notes in the final book, and what the
    /// unification review did (program dedupe + model verdicts).
    pub notes_written: usize,
    pub notes_dropped: usize,
    pub notes_edited: usize,
    /// Annotation resume hit-count (segments whose note decision was cached).
    pub notes_restored_from_cache: usize,
    /// The first provider error of the run, if any call failed — the
    /// difference between "8 failed" and knowing the endpoint said 401.
    pub sample_error: Option<String>,
}

fn meta_key(sig: &str) -> String {
    format!("expert_mem_{sig}")
}

fn anno_plan_key(sig: &str) -> String {
    format!("anno_plan_{sig}")
}

fn anno_mem_key(sig: &str) -> String {
    format!("anno_mem_{sig}")
}

fn anno_reviewed_key(sig: &str) -> String {
    format!("anno_reviewed_{sig}")
}

#[derive(Debug, Clone, Default)]
pub struct RenderSummary {
    pub restored_from_cache: usize,
    pub missing: usize,
    pub total_segments: usize,
    /// Segments whose annotation DECISION was restored from the cache —
    /// including the deliberate "no note here" skips. A resume/coverage
    /// metric, NOT the number of visible notes (a fully-annotated book
    /// prefills every segment but may carry only a handful of notes).
    pub note_segments_prefilled: usize,
    /// Actual visible notes (non-skip) in the rendered book — the number of
    /// `etc-note` blocks the reader will see.
    pub notes_in_output: usize,
}

/// Offline re-render: fill targets — and, given an annotation signature, notes —
/// purely from the cache, **never** calling an LLM (the absence of a `provider`
/// parameter is the type-level proof of zero cost). Unmatched segments keep
/// `target = None` and the format layer falls back to the source text. Lets the
/// user change layout/bilingual/colour for free. `sig: None` restores no
/// translations (annotate-only re-render keeps the book untranslated).
pub fn render_from_cache(
    book: &mut Book,
    store: &JobStore,
    sig: Option<&str>,
    anno_sig: Option<&str>,
) -> Result<RenderSummary> {
    let total = book.total_segments();
    let restored = match sig {
        Some(sig) => store.prefill_from_cache(book, sig)?,
        None => 0,
    };
    let note_segments_prefilled = match anno_sig {
        Some(asig) => store.prefill_notes_from_cache(book, asig)?,
        None => 0,
    };
    let notes_in_output = book
        .chapters
        .iter()
        .flat_map(|c| c.segments.iter())
        .filter(|s| s.note.as_ref().is_some_and(|n| !n.is_skip()))
        .count();
    Ok(RenderSummary {
        restored_from_cache: restored,
        missing: total - restored,
        total_segments: total,
        note_segments_prefilled,
        notes_in_output,
    })
}

/// Translate a whole book. Owns cache prefill, checkpointing and (for expert)
/// the pre-scan + consistency passes. When `cfg.annotations` is set, each
/// chapter is annotated right after its translation checkpoints, and the
/// book-wide review runs at the end. Reports progress via `on`.
pub async fn run(
    provider: &Provider,
    cfg: &TranslateConfig,
    book: &mut Book,
    store: &JobStore,
    sig: &str,
    on: impl FnMut(RunProgress) + Send,
) -> Result<RunSummary> {
    run_inner(provider, cfg, book, store, sig, true, on).await
}

/// Annotate-only run: no translation pass at all (the output keeps the source
/// text, plus notes). Requires `cfg.annotations`.
pub async fn annotate_only(
    provider: &Provider,
    cfg: &TranslateConfig,
    book: &mut Book,
    store: &JobStore,
    on: impl FnMut(RunProgress) + Send,
) -> Result<RunSummary> {
    if cfg.annotations.is_none() {
        return Err(crate::error::CoreError::Other(
            "annotate_only requires cfg.annotations".into(),
        ));
    }
    // The translation signature is never consulted when do_translate=false.
    run_inner(provider, cfg, book, store, "", false, on).await
}

async fn run_inner(
    provider: &Provider,
    cfg: &TranslateConfig,
    book: &mut Book,
    store: &JobStore,
    sig: &str,
    do_translate: bool,
    mut on: impl FnMut(RunProgress) + Send,
) -> Result<RunSummary> {
    let total_ch = book.chapters.len();
    let mut total = Stats::default();
    let mut anno_stats = Stats::default();

    // Annotations: restore every cached per-segment decision (notes AND
    // deliberate skips) so resume costs zero tokens.
    let anno_sig = cfg.annotation_signature();
    let mut notes_restored = 0usize;
    if let Some(asig) = anno_sig.as_deref() {
        notes_restored = store.prefill_notes_from_cache(book, asig)?;
        // Recorded so cache-only re-renders (CLI + desktop) can restore notes
        // without re-deriving the annotation config.
        store.set_meta("anno_sig", asig)?;
    }

    let restored = if do_translate {
        store.prefill_from_cache(book, sig)?
    } else {
        0
    };

    // Sentence level: every streamed batch is final, so checkpoint it to the
    // cache immediately. Expert streams intermediate (draft) batches that later
    // passes overwrite — caching those mid-flight would let a crash resume with
    // un-reflected drafts, so it keeps the chapter-boundary checkpoint only.
    let persist_batches = cfg.level != Level::Expert;

    // Expert: load cached memory or run the pre-scan (Pass 0).
    let mut mem: Option<ExpertMemory> = None;
    if do_translate && cfg.level == Level::Expert {
        on(RunProgress::at(Phase::Prescan, 0, total_ch));
        let m = match store.get_meta(&meta_key(sig))? {
            Some(j) => serde_json::from_str(&j).unwrap_or_default(),
            None => {
                let (m, stats) = expert::prescan(provider, cfg, book).await?;
                total.merge(&stats);
                store.set_meta(&meta_key(sig), &serde_json::to_string(&m)?)?;
                m
            }
        };
        mem = Some(m);
    }

    // Annotations: load the cached plan + rolling memory, or run Pass N0.
    let mut anno_ctx: Option<(crate::annotate::AnnotationPlan, crate::annotate::AnnoMemory)> = None;
    if let (Some(anno), Some(asig)) = (cfg.annotations.as_ref(), anno_sig.as_deref()) {
        on(RunProgress::at(Phase::AnnotatePlan, 0, total_ch));
        let plan = match store.get_meta(&anno_plan_key(asig))? {
            Some(j) => serde_json::from_str(&j).unwrap_or_default(),
            None => {
                let (p, stats) = crate::annotate::plan(provider, cfg, anno, book).await?;
                anno_stats.merge(&stats);
                store.set_meta(&anno_plan_key(asig), &serde_json::to_string(&p)?)?;
                p
            }
        };
        let amem: crate::annotate::AnnoMemory = match store.get_meta(&anno_mem_key(asig))? {
            Some(j) => serde_json::from_str(&j).unwrap_or_default(),
            None => Default::default(),
        };
        anno_ctx = Some((plan, amem));
    }

    for ci in 0..total_ch {
        // Publisher apparatus is never sent to a model: it is the page that
        // states the work may not be reproduced, it is worth nothing to a
        // reader in translation, and it is pure token cost. It stays in the
        // output byte-identical.
        if book.chapters[ci].apparatus {
            store.set_chapter_status(ci, &book.chapters[ci].href, "done")?;
            let mut p = RunProgress::at(Phase::Translating, ci, total_ch);
            p.href = book.chapters[ci].href.clone();
            on(p);
            continue;
        }
        store.set_chapter_status(ci, &book.chapters[ci].href, "in_progress")?;
        let href_for_batch = book.chapters[ci].href.clone();

        if do_translate {
            let on_ref: &mut (dyn FnMut(RunProgress) + Send) = &mut on;
            let mut on_batch = |batch: BatchPairs| {
                if persist_batches && !batch.cacheable.is_empty() {
                    // Best-effort: a cache write failure must not abort translation
                    // (the chapter-boundary checkpoint is the backstop).
                    let _ = store.cache_put_batch(&batch.cacheable, sig);
                }
                let mut p = RunProgress::at(Phase::Batch, ci, total_ch);
                p.href = href_for_batch.clone();
                p.pairs = batch.pairs;
                on_ref(p);
            };
            let stats = if let Some(m) = mem.as_mut() {
                let s = expert::translate_chapter_expert(
                    provider,
                    cfg,
                    &mut book.chapters[ci],
                    m,
                    &mut on_batch,
                )
                .await?;
                // persist growing memory (summary) for resume
                store.set_meta(&meta_key(sig), &serde_json::to_string(m)?)?;
                s
            } else {
                translate_chapter(provider, cfg, &mut book.chapters[ci], &mut on_batch).await?
            };
            total.merge(&stats);
            store.store_chapter(&book.chapters[ci], sig, &stats.failed_units)?;

            let sample = book.chapters[ci]
                .segments
                .iter()
                .find_map(|s| s.target.clone())
                .map(|t| t.chars().take(120).collect::<String>());

            // Pairs now stream per batch (Phase::Batch); the chapter event
            // carries none to avoid double-rendering in read-along UIs.
            let mut p = RunProgress::at(Phase::Translating, ci, total_ch);
            p.href = book.chapters[ci].href.clone();
            p.sample = sample;
            p.units_translated = stats.units_translated;
            p.units_failed = stats.units_failed;
            on(p);
        }

        // Annotation pass N1 for this chapter — after the translation
        // checkpoint, so a crash between the two never loses translated text.
        if let Some((plan, anno_mem)) = anno_ctx.as_mut() {
            let anno = cfg.annotations.as_ref().expect("anno_ctx implies config");
            let asig = anno_sig.as_deref().expect("anno_ctx implies signature");
            let href = book.chapters[ci].href.clone();
            let on_ref: &mut (dyn FnMut(RunProgress) + Send) = &mut on;
            let mut on_note_batch = |triples: Vec<(usize, String, crate::document::Note)>| {
                // Persist EVERY decision (including deliberate skips) so resume
                // is free. Best-effort, same rationale as translation batches.
                let keyed: Vec<(String, String)> = triples
                    .iter()
                    .map(|(bi, src, note)| {
                        (
                            crate::job::note_cache_key(&href, *bi, src, asig),
                            crate::job::encode_note_value(note),
                        )
                    })
                    .collect();
                let _ = store.cache_put_raw_batch(&keyed);
                let notes: Vec<(String, crate::document::Note)> = triples
                    .into_iter()
                    .filter(|(_, src, note)| !note.is_skip() && has_visible_text(src))
                    .map(|(_, src, note)| (src, note))
                    .collect();
                if !notes.is_empty() {
                    let mut p = RunProgress::at(Phase::Annotating, ci, total_ch);
                    p.href = href.clone();
                    p.notes = notes;
                    on_ref(p);
                }
            };
            let stats = crate::annotate::annotate_chapter(
                provider,
                cfg,
                anno,
                plan,
                anno_mem,
                &mut book.chapters[ci],
                &mut on_note_batch,
            )
            .await?;
            store.set_meta(&anno_mem_key(asig), &serde_json::to_string(anno_mem)?)?;
            anno_stats.merge(&stats);

            // Chapter-boundary event with this chapter's note counters.
            let mut p = RunProgress::at(Phase::Annotating, ci, total_ch);
            p.href = book.chapters[ci].href.clone();
            p.units_translated = stats.units_translated;
            p.units_failed = stats.units_failed;
            on(p);
        }

        store.set_chapter_status(ci, &book.chapters[ci].href, "done")?;
    }

    // Annotation stats fold in as tokens + failures only: `units_translated`
    // stays a TRANSLATION count (notes have their own `notes_written`).
    let mut summary = RunSummary {
        restored_from_cache: restored,
        tokens_in: total.tokens_in + anno_stats.tokens_in,
        tokens_out: total.tokens_out + anno_stats.tokens_out,
        units_translated: total.units_translated,
        units_failed: total.units_failed + anno_stats.units_failed,
        notes_restored_from_cache: notes_restored,
        sample_error: total
            .sample_error
            .clone()
            .or_else(|| anno_stats.sample_error.clone()),
        ..Default::default()
    };

    // Expert: consistency report (Pass 3).
    if let Some(m) = &mem {
        on(RunProgress::at(Phase::Consistency, total_ch, total_ch));
        summary.glossary_size = m.glossary.len();
        summary.inconsistencies = expert::consistency_report(book, m);
    }

    // Annotation pass N2: one book-wide unification review, exactly once per
    // annotation signature (the reviewed flag keeps a finished book's re-run at
    // zero LLM calls).
    if let (Some(anno), Some(asig)) = (cfg.annotations.as_ref(), anno_sig.as_deref()) {
        if store.get_meta(&anno_reviewed_key(asig))?.is_none() {
            on(RunProgress::at(Phase::AnnotateReview, total_ch, total_ch));
            let (outcome, stats) = crate::annotate::review(provider, anno, book).await?;
            summary.tokens_in += stats.tokens_in;
            summary.tokens_out += stats.tokens_out;
            summary.notes_dropped = outcome.dropped;
            summary.notes_edited = outcome.edited;
            // Re-persist every reviewed decision so cache-only re-renders and
            // resumes reflect the unified book, not the pre-review drafts.
            let mut keyed: Vec<(String, String)> = Vec::new();
            for chapter in &book.chapters {
                for seg in &chapter.segments {
                    if let Some(note) = &seg.note {
                        keyed.push((
                            crate::job::note_cache_key(
                                &chapter.href,
                                seg.block_index,
                                &seg.source,
                                asig,
                            ),
                            crate::job::encode_note_value(note),
                        ));
                    }
                }
            }
            store.cache_put_raw_batch(&keyed)?;
            store.set_meta(&anno_reviewed_key(asig), "done")?;
        }
        summary.notes_written = book
            .chapters
            .iter()
            .flat_map(|c| c.segments.iter())
            .filter(|s| s.note.as_ref().is_some_and(|n| !n.is_skip()))
            .count();
    }
    Ok(summary)
}

/// Translate one chapter's still-untranslated segments at sentence level.
pub async fn translate_chapter(
    provider: &Provider,
    cfg: &TranslateConfig,
    chapter: &mut crate::document::Chapter,
    on_batch: PairSink<'_>,
) -> Result<Stats> {
    let pending: Vec<(u64, String)> = chapter
        .segments
        .iter()
        .filter(|s| s.target.is_none())
        .map(|s| (s.block_index as u64, s.source.clone()))
        .collect();
    if pending.is_empty() {
        return Ok(Stats::default());
    }
    let mut stats = Stats::default();
    let system = prompt::sentence_system(cfg);
    let map = translate_unit_map(provider, cfg, pending, &system, &mut stats, Some(on_batch)).await;
    for seg in &mut chapter.segments {
        if seg.target.is_none() {
            if let Some(t) = map.get(&(seg.block_index as u64)) {
                seg.target = Some(t.clone());
            }
        }
    }
    Ok(stats)
}

/// Translate a list of `(id, text)` units under a given system prompt, batching
/// by caps and applying the three-tier fallback. Returns id → translation
/// (failed single units fall back to the source text). Shared by sentence and
/// expert (draft + reflect) passes.
pub(crate) async fn translate_unit_map(
    provider: &Provider,
    cfg: &TranslateConfig,
    units: Vec<(u64, String)>,
    system: &str,
    stats: &mut Stats,
    mut on_batch: Option<PairSink<'_>>,
) -> HashMap<u64, String> {
    // Initial batches honouring unit + token caps.
    let mut work: Vec<Vec<(u64, String)>> = Vec::new();
    let mut cur: Vec<(u64, String)> = Vec::new();
    let mut cur_tokens = 0usize;
    for (id, src) in units {
        let t = (src.chars().count() as f32 / 2.5).ceil() as usize;
        if !cur.is_empty()
            && (cur.len() >= cfg.max_batch_sentences || cur_tokens + t > cfg.max_chunk_tokens)
        {
            work.push(std::mem::take(&mut cur));
            cur_tokens = 0;
        }
        cur.push((id, src));
        cur_tokens += t;
    }
    if !cur.is_empty() {
        work.push(cur);
    }
    // Batches run in book order, up to `concurrency` at a time. Each batch owns
    // its three-tier fallback (whole → split halves → single → source) inside
    // `translate_batch`, so a failure never blocks its siblings. Streaming and
    // stats are applied sequentially per concurrency window, in book order, so
    // the read-along stays ordered and `on_batch` (a `&mut FnMut`) is never
    // touched from two tasks at once.
    let concurrency = cfg.concurrency.max(1);
    let mut results: HashMap<u64, String> = HashMap::new();
    for window in work.chunks(concurrency) {
        let outcomes =
            futures::future::join_all(window.iter().map(|b| translate_batch(provider, b, system)))
                .await;
        for outcome in outcomes {
            stats.merge(&outcome.stats);
            if let Some(cb) = on_batch.as_deref_mut() {
                if !outcome.pairs.is_empty() || !outcome.cacheable.is_empty() {
                    cb(BatchPairs {
                        pairs: outcome.pairs,
                        cacheable: outcome.cacheable,
                    });
                }
            }
            results.extend(outcome.map);
        }
    }
    results
}

/// One translated batch: the id→target map, the (source, target) pairs to stream
/// (visible-text only), and the stats it accrued.
struct BatchOutcome {
    map: HashMap<u64, String>,
    pairs: Vec<(String, String)>,
    /// See `BatchPairs::cacheable`.
    cacheable: Vec<(String, String)>,
    stats: Stats,
}

/// Translate one batch with the three-tier fallback, self-contained so batches
/// can run concurrently. On a multi-unit failure it splits and recurses; a
/// single-unit failure falls back to the source text (and counts as failed).
fn translate_batch<'a>(
    provider: &'a Provider,
    batch: &'a [(u64, String)],
    system: &'a str,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = BatchOutcome> + Send + 'a>> {
    Box::pin(async move {
        let mut stats = Stats::default();
        match translate_once(provider, batch, system, &mut stats).await {
            Ok(map) => {
                stats.units_translated += map.len();
                let mut pairs: Vec<(String, String)> = batch
                    .iter()
                    .filter_map(|(id, src)| map.get(id).map(|t| (src.clone(), t.clone())))
                    .collect();
                pairs.retain(|(src, _)| has_visible_text(src));
                let cacheable = pairs.clone();
                BatchOutcome {
                    map,
                    pairs,
                    cacheable,
                    stats,
                }
            }
            Err(_) if batch.len() > 1 => {
                let mid = batch.len() / 2;
                let a = translate_batch(provider, &batch[..mid], system).await;
                let b = translate_batch(provider, &batch[mid..], system).await;
                let mut map = a.map;
                map.extend(b.map);
                let mut pairs = a.pairs;
                pairs.extend(b.pairs);
                let mut cacheable = a.cacheable;
                cacheable.extend(b.cacheable);
                stats.merge(&a.stats);
                stats.merge(&b.stats);
                BatchOutcome {
                    map,
                    pairs,
                    cacheable,
                    stats,
                }
            }
            Err(e) => {
                let (id, src) = &batch[0];
                stats.units_failed += 1;
                stats.failed_units.insert(*id);
                stats.sample_error.get_or_insert_with(|| e.to_string());
                let mut map = HashMap::new();
                map.insert(*id, src.clone());
                // A failed unit falls back to its source in the output, so stream
                // that same (src, src) pair to the read-along too. Without it the
                // preview slot for this segment never fills, so the page keeps
                // `pending > 0` and the frontier ("回到正在翻譯") gets stuck on it
                // forever even as later pages finish.
                let pairs = if has_visible_text(src) {
                    vec![(src.clone(), src.clone())]
                } else {
                    Vec::new()
                };
                BatchOutcome {
                    map,
                    pairs,
                    cacheable: Vec::new(),
                    stats,
                }
            }
        }
    })
}

/// One LLM call for a batch under `system`: build payload, call (retry), validate.
async fn translate_once(
    provider: &Provider,
    batch: &[(u64, String)],
    system: &str,
    stats: &mut Stats,
) -> Result<HashMap<u64, String>> {
    let units: HashMap<u64, String> = batch.iter().cloned().collect();
    let sentences: Vec<_> = batch
        .iter()
        .map(|(id, text)| json!({ "id": id, "text": text }))
        .collect();
    let payload = json!({ "sentences": sentences }).to_string();

    let req = CompletionRequest {
        system: system.to_string(),
        messages: vec![ChatMessage::user(payload.clone())],
        temperature: 0.2,
    };

    let resp = provider.complete_retrying(&req, 2).await?;
    stats.tokens_in += resp.tokens_in;
    stats.tokens_out += resp.tokens_out;

    match validate::parse_and_check(&resp.text, &units) {
        Ok(map) => Ok(map),
        // One targeted repair before giving up. The common case is inline
        // emphasis on a word the target language does not spell out —
        // "there ⟦2⟧is⟦/2⟧ one right way" has nothing to italicise in Chinese —
        // so the model returns a perfectly good sentence with the pair
        // dropped. Failing there costs the whole paragraph: it falls back to
        // its source, and the reader gets one English paragraph in a
        // translated book. Naming what went missing recovers it; re-rolling
        // the same prompt does not, which is why this is not just more retries.
        Err(crate::CoreError::Validation(why)) if why.contains("placeholder mismatch") => {
            let repair = CompletionRequest {
                system: system.to_string(),
                messages: vec![
                    ChatMessage::user(payload),
                    ChatMessage::assistant(resp.text.clone()),
                    ChatMessage::user(format!(
                        "That response was rejected: {why}. Every ⟦…⟧ marker in a \
                         sentence must also appear in its translation — the same \
                         markers, the same number of times, in reading order. A \
                         paired marker wraps the words the source emphasised: put \
                         it around whatever carries that emphasis in the \
                         translation, or around the nearest equivalent phrase. \
                         Never drop one. Reply with the same JSON array and \
                         nothing else."
                    )),
                ],
                temperature: 0.0,
            };
            let resp2 = provider.complete_retrying(&repair, 1).await?;
            stats.tokens_in += resp2.tokens_in;
            stats.tokens_out += resp2.tokens_out;
            validate::parse_and_check(&resp2.text, &units)
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TranslateConfig;
    use crate::llm::{mock::MockProvider, Provider};

    async fn run_units(concurrency: usize) -> (HashMap<u64, String>, usize, Stats) {
        let provider = Provider::Mock(MockProvider);
        let mut cfg = TranslateConfig::new("繁體中文");
        cfg.max_batch_sentences = 4; // force several batches from 23 units
        cfg.concurrency = concurrency;
        let units: Vec<(u64, String)> = (0..23u64).map(|i| (i, format!("line {i}"))).collect();
        let mut stats = Stats::default();
        let mut streamed = 0usize;
        let mut sink = |batch: BatchPairs| streamed += batch.pairs.len();
        let map =
            translate_unit_map(&provider, &cfg, units, "sys", &mut stats, Some(&mut sink)).await;
        (map, streamed, stats)
    }

    /// A dropped inline pair must cost one extra call, not the paragraph.
    /// Without the repair the unit fails validation and falls back to its
    /// source, which is how one English paragraph ends up in a Chinese book.
    #[tokio::test]
    async fn a_dropped_inline_pair_is_repaired_instead_of_failing_the_unit() {
        use crate::llm::mock::DROP_PAIRS_TOKEN;
        let provider = Provider::Mock(MockProvider);
        let cfg = TranslateConfig::new("繁體中文");
        let src = format!("there ⟦2⟧is⟦/2⟧ one right way {DROP_PAIRS_TOKEN}");
        let mut stats = Stats::default();
        let map = translate_unit_map(
            &provider,
            &cfg,
            vec![(0u64, src.clone())],
            "sys",
            &mut stats,
            None,
        )
        .await;

        assert_eq!(stats.units_failed, 0, "the repair must rescue the unit");
        assert!(stats.failed_units.is_empty());
        let got = map.get(&0).expect("unit translated");
        assert!(
            got.contains("⟦2⟧") && got.contains("⟦/2⟧"),
            "the repaired translation must carry the pair back: {got}"
        );
    }

    // Concurrency must change throughput, never the result: parallel output is
    // byte-identical to sequential, all units land, and every visible unit
    // streams exactly once.
    #[tokio::test]
    async fn concurrency_matches_sequential() {
        let (seq_map, seq_streamed, seq_stats) = run_units(1).await;
        let (par_map, par_streamed, par_stats) = run_units(5).await;

        assert_eq!(seq_map.len(), 23, "all units translated");
        assert_eq!(seq_map, par_map, "parallel result must equal sequential");
        assert_eq!(seq_map.get(&7).map(String::as_str), Some("〈譯〉line 7"));
        assert_eq!(seq_streamed, 23, "every unit streams once (sequential)");
        assert_eq!(par_streamed, 23, "every unit streams once (parallel)");
        assert_eq!(seq_stats.units_translated, par_stats.units_translated);
        assert_eq!(seq_stats.units_failed, 0);
    }
}
