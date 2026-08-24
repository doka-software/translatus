# Annotation quality tuning map (ANNOTATION-TUNING)

One page answering "I want to change X about the margin notes → edit where".
Read this before iterating on note quality; no archaeology required.

Pipeline (the module comment at the top of `crates/core/src/annotate/mod.rs`
is the authoritative version):

```
N0 plan (whole-book sampling → topic map) → N-sel select (per-chapter spots, hard cap in code)
→ N1 notes (written only for selected paragraphs) → N2 review (whole-book keep/edit/drop)
```

## What to change → where

| What you want to change | Where | Notes |
|---|---|---|
| Note density / per-chapter cap formula | `annotate/mod.rs` `chapter_note_cap()` (Sparse n/24, Medium n/12, Rich n/6; absolute cap `CHAPTER_NOTE_ABS_MAX = 12`) | Hard cap **in code** — the model cannot exceed it however greedy. If you touch it, update the `chapter_note_cap_formula` test and the estimate's note_ratio (CLI `estimate_numbers` + app `estimate_annotation`) |
| Spot-selection quality (where to pause, from what angle) | `annotate/prompt.rs` `select_system()` | The selection prompt picks spots only, writes nothing; density wording lives in `density_line()` |
| Service menu (chips / `--note-presets` — the ONE "what should notes do for you" layer) | `annotate/prompt.rs` `PRESETS` table (id → guidance sentence) + `presets_block()` | Nine fixed ids: terms/history/author/culture/characters/concepts/world/methods/research. Injected into plan + selection + writing prompts (after the reader's free text); unknown ids warn and are ignored; the canonical set enters `annotation_signature`. Free-text profile is optional when ≥1 service is ticked (`profile_line()` fallback). 2026-08-15: a short-lived separate "goals" axis was removed — it duplicated this layer |
| Explanation level (講給誰聽 / `--note-level` / profile doc `level`) | `config.rs` `ExplainLevel` + `annotate/prompt.rs` `level_block()` | beginner (everyday language + examples, selection may pause more for newcomers) / general (default, injects nothing) / insider (skip anything an insider knows; 寧缺勿濫). Enters `annotation_signature` |
| Cognitive anchors (認知錨 / `--note-anchors` / profile doc `anchors`) | `annotate/prompt.rs` `canonical_anchors()` + `anchors_block()`; caps `ANCHOR_MAX_COUNT = 16`, `ANCHOR_MAX_CHARS = 80` | Short labels of what the reader already knows; injected into plan + selection + writing prompts as bridging material ("explain FROM familiar ground; an inaccurate analogy is worse than none"). Canonical list enters `annotation_signature` |
| Voice register (`--note-voice` study\|companion) | `config.rs` `NoteVoice` + `annotate/prompt.rs` `COMPANION_NOTE_STYLE` / `default_style_for()` | Picks which default style paragraph applies when the user has no custom style; a custom style overrides either voice. Hard rules identical in both registers. Enters `annotation_signature` |
| Thread map (全書線索圖: backward-reference planning) | plan prompt `threads` output (`annotate/prompt.rs` `plan_system()`), carried by `AnnotationPlan::threads` → `as_prompt_block()`; selection instruction in `select_system()` | Gives the per-chapter selection pass forward visibility so notes are PLACED after an insight and reach back — threads never let note text preview unread content (hard rule 7) |
| Reader-boundary output check (the Fable rule) | `annotate/prompt.rs` `note_addresses_reader()` wired into `note_text_ok()` (`annotate/mod.rs`) | Program-side scan of every accepted note (N1 write + N2 edit): reader-addressing phrasings ("就像你…", "dear reader"…) are rejected like a placeholder leak. Keep the phrase list conservative — a hit must never be a keepable note |
| Generic-opener check (具體開頭律) | `annotate/prompt.rs` `note_opens_generic()`, same `note_text_ok()` chokepoint | A note opening with "這段…/此處…/this passage…" framing is filler by construction and is rejected; quoted openers pass. Found in the 2026-08-15 blind evaluation and promoted from prompt guidance to machine check |
| Reader-profile contract (agent-fillable document) | `config.rs` `ReaderProfile` (+ CLI `--note-profile`, MCP `note_profile` inline-JSON-only) | One JSON doc = purpose / anchors / presets / voice / lang / density / style; explicit flags override; unknown keys are a hard parse error. Schema + the standard prompt for the reader's own AI: `docs/READER-PROFILE.md` |
| Selection input volume | `annotate/mod.rs` `SELECT_SNIPPET_CHARS` (200 chars/paragraph), `SELECT_WINDOW_CHARS` (12k/window) | Bigger = selection sees more, costs more |
| Writing quality (tone, depth, length) | **default style paragraph** `annotate/prompt.rs` `DEFAULT_NOTE_STYLE`; users can override the whole paragraph via `AnnotationConfig::style` (CLI `--note-style`, app Settings → Prompt → note style) | The 160-char length target lives here (`NOTE_MAX_CHARS` is just the constant's source) — it is **not** a hard rule |
| The non-negotiable content contract | `annotate/prompt.rs` `hard_rules()` (neutral; never address the reader; no meta book-reviewing; notes self-identify; no paraphrasing; no placeholders/HTML) | N1 and N2 share the same copy; user style cannot override it. Editing this = editing the product contract — be deliberate |
| Context volume when writing | `annotate/mod.rs` `GEN_CONTEXT_CHARS` (120 chars of each neighbour paragraph) | |
| N2 review criteria (dedupe, rewrite) | `annotate/prompt.rs` `review_system()`; batching `REVIEW_BATCH = 60`, context truncation `REVIEW_CONTEXT_CHARS = 80` | Code does an exact-dedupe first (inside `review()`); the model only rules keep/edit/drop; an edit cannot move a note's position (AN-014) |
| Cross-chapter memory (repetition guard) | `annotate/mod.rs` `AnnoMemory` (noted_topics cap `TOPICS_CAP = 120`, digest cap `DIGEST_CAP_CHARS = 1600`) | |
| Whole-book plan sampling volume | `annotate/mod.rs` `PLAN_CHARS_PER_CHAPTER` (900 head) + `PLAN_MID_CHARS` (400 mid-chapter slice), `PLAN_TOTAL_CHARS` (32k) | Head + mid sample per chapter so the thread map sees development, not only openings; mock/failure always falls back to a stub plan; never blocks the run |

## How the style layer works (mirrors translation's locked/editable split)

- `AnnotationConfig.style: Option<String>` (serde default) → `style_block()`
  goes through `translate::prompt::sanitize_style` (4000-char cap) and is
  injected into the N1 and N2 prompts as the "note style (customisable)"
  section; `None`/blank → `DEFAULT_NOTE_STYLE`.
- Selection (N-sel) and planning (N0) do **not** consume style — they produce
  no note prose.
- **Signatures**: style enters `annotation_signature()` (config.rs), so a
  style change re-runs the notes while the translation cache
  (`cache_signature()`) is untouched. When prompt text changes materially,
  bump the signature salt (currently `inkferry-anno-v7`) and add a salt-bump
  test.

## Mock behaviour (how tests run the full pipeline at zero cost)

`llm/mock.rs`: selection picks the first of every 3 paragraphs (before/after
alternating, descending priority); `SELECT_ALL_TOKEN` makes it greedily
select everything (for cap tests); N1 writes each selected paragraph a
`〈註〉背景補充: <head>` stub; N2 drops exact-text duplicates. The e2e lives
in `crates/core/tests/annotate_e2e.rs`.

## Verification checklist (run after changes)

```
cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

Changed a CLI flag / config field → update `CAPABILITIES.toml` in the same
change (the parity tests turn red otherwise).
