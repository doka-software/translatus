# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.1.0] - 2026-08-18

### Added

- `translatus mcp install` / `mcp uninstall`: registers the MCP server with the
  agent clients found on this machine (Claude Code, Codex) through each one's
  own CLI rather than by editing its configuration file. The interactive session
  offers to do it once on first run, so a normal install is one step; package
  installation itself never writes into another program's config.

## [1.0.0] - 2026-08-17

First public release.

### Security

- The Codex and Claude subscription children run with a reduced environment,
  every built-in tool disabled, and no session persistence; a contextual
  trusted-book reminder is shown when users choose Codex subscription mode.
- Hardened bearer-token redaction, redirect handling, settings-file permissions,
  MCP write paths, CLI path collision checks, and release-workflow permissions.
- Client disconnects cancel the provider call, and the sidecar serves one paid
  request at a time.
- Final book output is an owner-only atomic replacement, target-language
  filename traversal is blocked, and MCP-selected loopback endpoints never see
  environment or OS-keychain credentials.

### Added
- **Reader-profile contract** (`--note-profile`, MCP `note_profile`): one JSON
  document — purpose, cognitive anchors, presets, voice, language, density,
  style — that a reader or the reader's own AI assistant fills to steer the
  margin notes. Schema and the standard hand-to-your-AI prompt:
  `docs/READER-PROFILE.md`. Explicit flags override document fields; over MCP
  only inline JSON is accepted (file paths would read arbitrary server paths).
- **Service menu & explanation level.** The preset help angles grew into the
  single "what should the notes do for you" layer — three new services join
  the six existing ones: `world` (connect passages to the real world: later
  developments, other fields, historical parallels — verified facts only),
  `methods` (unpack methods with their conditions, costs and limits) and
  `research` (prioritise citable facts, sources, the book's structure, and
  points of dispute). Picking is enough: with at least one service ticked the
  free-text profile becomes optional, and the interactive session leads with
  the picker, demoting free text to "In your own words". A new
  `--note-level beginner|general|insider` dial sets how much the notes
  assume: beginner explains in everyday language with examples; insider only
  adds what an insider could not easily look up. The session also collects
  anchors ("You already know") and the voice register.
- **Cognitive anchors** (`--note-anchors`): short labels of what the reader
  already knows; notes explain new concepts by bridging from that familiar
  ground (with an accuracy guard: a wrong analogy is worse than none), and are
  still never allowed to mention the reader.
- **Note voice registers** (`--note-voice study|companion`): the restrained
  study register stays the default; the companion register writes in a
  friend-at-your-side tone with a higher share of short reaction notes. Hard
  rules (neutrality, no reader-addressing, no spoilers) identical in both.
- **Book-wide thread map**: the planning pass now also samples mid-chapter text
  and returns cross-chapter threads (concept first appears → pays off), so the
  per-chapter selection pass can place notes AFTER an insight lands and reach
  back to already-read chapters instead of discovering the connection too late.
  Note text still never previews unread content.
- **Reader-boundary output check**: every accepted note (writing and review
  passes) is program-scanned for reader-addressing phrasings; violations are
  rejected and rewritten, making "the notes never talk about you" a machine
  guarantee rather than a prompt promise.

### Changed
- Annotation cache signature salt bumped (v6 → v7): existing note caches
  re-annotate under the new prompts; translation caches are untouched and are
  never re-billed.

### Added
- **Interactive session.** Running `translatus` with no subcommand on a
  terminal opens a guided flow: it finds the books around you, asks how they
  should read, quantifies the run (chapters, characters, estimated cost), waits
  for an explicit confirmation before spending anything, and paints a live
  progress board with per-chapter status and a remaining-time estimate
  extrapolated from measured throughput. It prints the equivalent command as
  you go, so the flags stay learnable. In a pipe or a script a bare
  `translatus` prints usage exactly as before, and the `--json` agent contract
  is unchanged — the interactive path builds the same arguments and calls the
  same engine. The session speaks five languages (English, 繁體中文, 简体中文,
  日本語, 한국어), following the terminal locale with a `TRANSLATUS_LANG`
  override; it offers real model sources only (subscription sidecar, API key,
  or Ollama — the Ollama model list is read live from `ollama list`), while
  the `mock` provider stays available behind the `--provider` flag for
  offline format checks. In subscription mode the Settings screen takes the
  sidecar's printed access token (stored in the OS keychain like a key) and
  accepts a sidecar on any loopback port, not just the default `8765`.
- **MCP server** (`translatus mcp`): a stdio Model Context Protocol server so
  agents (Claude Code, or any MCP client) can call `estimate_book`,
  `translate_book`, and `annotate_book` as first-class tools. Results reuse
  the `--json` schema by construction (each call self-execs the CLI in
  `--json` mode); long calls stream per-chapter `notifications/progress`;
  re-calling a tool with the same arguments resumes from the job cache.
  Registration: `claude mcp add translatus -- translatus mcp`. Covered by
  e2e tests (handshake, tools/list, mock estimate/translate with progress)
  and declared in CAPABILITIES.toml (`mcp-server`, CLI-only with reason).

- **Human-mode CLI experience**: a cost-estimate
  line before every real run (`estimate` logic reused; `--json` untouched), a
  fail-fast setup card when a hosted provider has no API key (subscription
  sidecar / API key / Ollama — the same three choices the interactive
  launch), a mock-provider dry-run notice, per-chapter and per-note progress
  lines, a completion summary (segments, notes written/edited/dropped,
  tokens, cost, resume tip), and a Ctrl-C message explaining that re-running
  the same command resumes without re-billing. `translatus --help` now ends
  with a provider setup card; `estimate` prints a human summary without
  `--json`. All CLI messages are English.
- **docs/GUIDE.md and docs/FAQ.md**: a full user guide (install, providers
  including the subscription sidecar, every flag with examples, cache/resume
  behavior, JSON mode for agents, troubleshooting) and an honest FAQ (costs,
  subscription compliance, funding model, data flow, prior-art
  differences). docs/ANNOTATION-TUNING.md is now in English.
- **Preset help angles for margin notes** (`--note-presets` chips): six
  fixed ids — `terms`, `history`, `author`, `culture`,
  `characters`, `concepts` — each mapping to one guidance sentence injected
  into the selection pass (angle guidance) and the writing pass, right after
  the reader's free-text profile. Unknown ids are ignored with a warning. The
  canonical preset set joins `annotation_signature` (tick a chip →
  re-annotate; the translation cache is never touched).
- Per-book annotation settings support in the shared `Settings` struct
  (`annotations.presets` / `annotations.density` global defaults +
  `annotations.books` per-book overrides) for per-book onboarding.

### Changed
- **Margin-note writing framework** landed in the annotation prompts:
  `DEFAULT_NOTE_STYLE` now encodes the ten craft rules (concrete first
  sentence, one note = one thing, length varies with content, judgements must
  anchor to specific words, the reader's likely misreading gets priority,
  claims must be checkable), the selection pass prioritises spots a reader of
  this background would misread or miss, and the review pass flags notes that
  are all the same length. Derived from a cross-tradition study of real
  annotators (金聖嘆／脂硯齋／毛宗崗, Gardner's *Annotated Alice*, high-voted
  Genius notes) and validated by a persona blind-review loop. A second pass
  added: a **no-forward-spoiler** rule (cross-chapter links may only look back
  at what the reader has already read, never reveal later plot), a
  **factual-accuracy** rule for real-world connections (a note may tie the text
  to the wider world — later history, other fields — but only with verifiable
  facts, never fabricated ones), selection guidance that **climaxes and famous
  scenes are not note-spots** and every note must map to a concrete reader need,
  and a **native-register** requirement (write in the target language's natural
  voice, no translationese). The annotation cache salt bumped through
  `inkferry-anno-v5` to **`inkferry-anno-v6`** (the prompt texts changed but do
  not enter the hash on their own); translation caches are untouched.
- **Public mode name: "fast" → "standard"** (App: 一般／一般／Standard／標準／
  표준 across the five UI languages). The default single-pass mode is now
  called *standard mode* everywhere user-facing; `--level standard` was
  already accepted and is now the documented spelling, and `一般` joins the
  accepted aliases. Nothing internal moved: the `sentence` level value, the
  cache signature, and the historical aliases (`fast`, `快速`…) all remain
  valid, so existing caches and scripts are untouched.
- **README**: leads with the engine/CLI; product positioning moved to the
  end and shrank to a short note. English docs now use English-reader
  examples (`--to English`); the zh-TW README keeps `--to "繁體中文"`.
- **Project rename: InkFerry → Translatus.** The open-source engine + CLI is
  now **Translatus** (Latin *translatus*, "carried across" — the root of
  *translate*). The CLI binary is `translatus`, the repository is
  `doka-software/translatus`.
  Cache/signature salts (`inkferry-anno-*`) keep their historical strings so
  no existing cache is invalidated by the rename; older entries below use the
  old name as written at the time.
- **Product term rename**: the zh-Hant product name for annotations is now
  眉批 (was 夾註); en "Margin notes", zh-Hans 批注, ja AI注釈, ko AI 주석
  (ja/ko revised 2026-07-19 after a native-corpus audit: in Japanese and
  Korean ebook UIs メモ/노트 mean the user's own notes, while 注釈/주석 are
  the native words for provided explanatory notes).
  Prompts, CLI messages and docs follow. Internal identifiers (`annotate`
  module, `etc-note` class, CLI subcommand/flags) and the in-book 「〔註〕」
  marker are unchanged. The annotation cache salt bumped to
  `inkferry-anno-v4` (prompt texts changed + presets joined the recipe);
  translation caches are untouched.
- **Repository split**: non-engine code moved out. This repository is the
  MIT-licensed engine (`et-core`) + CLI (`inkferry`), free forever, and is
  the single upstream any embedding host consumes as a pinned dependency.
- **Hostile EPUB entry names can no longer ride into the output.** Reading was
  always safe (entries are held in memory, never extracted), but the writer
  rebuilt the archive from the same names, so an entry called
  `../../../../tmp/x` survived translation verbatim. The translated book was
  therefore a zip-slip payload carried by a tool the reader trusts, and the
  next program to unpack it took the hit. Absolute and `..`-bearing names are
  now dropped when the book is read; OCF forbids them anyway, so nothing
  legitimate is affected.
- **The input file is size-capped before it is read.** Every other guard
  describes decompressed bytes and so could not fire until the file was
  already resident in memory. A large file with an `.epub` name — not even a
  valid archive — was a memory spike before the parser had an opinion.
- **Provider error bodies are scrubbed of credentials.** `--base-url` points at
  whatever OpenAI-compatible gateway you choose, and a gateway that echoes
  request headers in an error would put your API key into stderr and, over
  MCP, into an agent's context. Key-shaped tokens and bearer headers are
  redacted before the body reaches an error message.
- **A remote `base_url` must use HTTPS.** The rule existed and was tested but
  nothing called it; `--base-url http://…` would have sent your key in clear
  text. It is now enforced where every caller funnels through.
- **The job cache is created owner-only (`0600`) on Unix.** It stores
  translated passages and margin notes as plaintext and previously inherited
  the ambient umask, which on a shared machine can mean world-readable.
- **The subscription sidecar refuses to bind anywhere but loopback.** It was a
  default rather than a guarantee: `LLM_SUB_KIT_HOST` could move it onto the
  network, and requests without an `Origin` were trusted on the strength of
  the `Host` header, which any client can set to `localhost`. A non-loopback
  bind is now fatal at startup, and peer identity is read from the socket.

### Fixed
- **`translatus` exits non-zero when segments fail.** A run that failed every
  segment still printed `done:` and exited 0, so a script or CI job checking
  only the exit code read a total failure as success. The output file and the
  resumable cache are still written, and the `--json` summary is unchanged.
- **`--provider anthropic` is refused instead of silently failing.** It built
  an OpenAI client pointed at `api.anthropic.com`, which speaks
  `POST /v1/messages` with an `x-api-key` header rather than
  `/chat/completions` with a bearer token, so every request failed after the
  docs had promised it would work. It now fails immediately with a message
  pointing at the two routes that do work: an OpenAI-compatible gateway via
  `--provider openai --base-url`, or the subscription sidecar for Claude.
- MCP progress and results no longer echo book-controlled filenames, provider
  diagnostics, subprocess output, or model-authored text into the calling
  agent's context; only fixed status and numeric metadata cross that boundary.
- Cost estimates read `$0.00` for Claude Opus, Fable and Mythos models,
  which were never priced in the estimator. All Claude tiers are now priced
  (verified against the official pages 2026-07-21) and covered by a test that
  fails if any model the UI offers falls through to zero.

## Pre-public history

The versions below were the private line before this engine was open-sourced,
kept for the record. The public release numbering starts at 1.0.0 above.
