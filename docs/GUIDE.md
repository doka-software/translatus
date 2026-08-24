# Translatus User Guide

This guide walks through everything the `translatus` CLI can do, in the order
you are likely to need it: install, dry-run, price a book, connect a model,
translate, annotate, and recover from interruptions. It assumes you can use a
terminal and nothing else. The [FAQ](FAQ.md) answers the questions this guide
raises; the [README](../README.md) explains why the project exists.

### Table of contents

- [Installation](#installation)
- [The interactive session](#the-interactive-session)
- [First run: a free dry run](#first-run-a-free-dry-run)
- [Estimating cost before you spend](#estimating-cost-before-you-spend)
- [Connecting a model](#connecting-a-model)
- [Translating a book](#translating-a-book)
- [Margin notes](#margin-notes)
- [Cache, resume, and free re-export](#cache-resume-and-free-re-export)
- [JSON mode for scripts and agents](#json-mode-for-scripts-and-agents)
- [Use with your agent (MCP)](#use-with-your-agent-mcp)
- [Troubleshooting](#troubleshooting)
- [Common flags](#common-flags)

### Installation

Translatus is a Rust workspace; install from source with a stable toolchain
([rustup.rs](https://rustup.rs) if you need one):

```bash
git clone https://github.com/doka-software/translatus
cd translatus
cargo install --path apps/cli     # installs the `translatus` binary
```

Publishing to crates.io (plain `cargo install translatus`) is planned; until
then, source is the way. The optional subscription sidecar additionally needs
Node.js ≥ 20 — see [Connecting a model](#connecting-a-model).

### The interactive session

Run `translatus` with no arguments on a terminal and it opens an interactive
session: it lists the books around you (or in the folder you set in Settings),
walks you through translation and margin-note choices, quotes the work and the
cost, and only runs after you confirm. Everything it can do is also a flag —
it prints the equivalent command as you go.

The session speaks **English, 繁體中文, 简体中文, 日本語, and 한국어**. It
follows your terminal locale (`LANG` / `LC_ALL` / `LC_MESSAGES`); set
`TRANSLATUS_LANG` (e.g. `TRANSLATUS_LANG=zh_TW`) to override. This affects the
interface only — the language your *notes* are written in is its own setting
(`--note-lang`, or "Note language" in the session), and the translation target
is whatever you ask for.

Model sources offered in the session: a Codex/ChatGPT or Claude subscription
(via the local sidecar), your own API key, or a local Ollama model — the
Ollama model list is read live from `ollama list`. Choices are saved to a
local `settings.json` (point `TRANSLATUS_CONFIG_DIR` elsewhere for a separate
profile). In a pipe or a script, a bare `translatus` prints usage instead.

### First run: a free dry run

The default provider is `mock`: it runs the entire pipeline — parsing,
batching, placeholder protection, reassembly, EPUB writing — without calling
any model. Use it to confirm your book survives the round trip before
spending a cent:

```bash
$ translatus translate book.epub --to English
translating book.epub → English (Sentence); 12 chapters, 340 segments
note: using the built-in mock provider — an offline dry run that checks formatting, not a real translation. See `translatus --help` to connect a model.
  [1/12] text/ch001.xhtml — 24 translated, 0 failed
  ...
done: book.English.epub
  12 chapters · 340 segments — 340 translated, 0 failed, 0 from cache
  tokens 118402 in / 11840 out (mock)
job cache: book.English.etjob (re-running the same command resumes; cached segments are never re-billed)
```

Open the output in any reader: layout, images, and styles should be exactly
those of the original, with every text node wrapped in a mock marker.

### Estimating cost before you spend

`estimate` parses the book, counts tokens, and prices the run for a given
model — no model calls happen:

```bash
$ translatus estimate book.epub --to English --model gpt-4o-mini
book.epub → English (sentence): 12 chapters, 340 segments
  translation ~118402 tokens in / ~130242 out ≈ US$0.10 (gpt-4o-mini)
(rough numbers; actuals depend on the book and, for notes, the reader profile)
```

Add `--annotate` (and optionally `--density`) to include the margin-note
passes in the estimate. Human-mode `translate` and `annotate` also print this
estimate line automatically before starting, so a surprise bill is hard to
get. Models without built-in price data report tokens only.

### Connecting a model

Translatus never bundles a model or an account; you bring one. Three ways,
the classic three-way choice:

**API key** — for anyone with an OpenAI or OpenAI-compatible key.
Pay per use; a book costs from under a dollar to a few dollars depending on
the model.

```bash
export OPENAI_API_KEY=sk-...
translatus translate book.epub --to English --provider openai --model gpt-4o-mini
# Claude: not via --provider anthropic (not implemented yet — it would speak the
#   OpenAI wire format to an API that does not accept it). Use either an
#   OpenAI-compatible gateway (--provider openai --base-url ...) or the
#   subscription sidecar, which talks to Claude through the official SDK.
# Any OpenAI-compatible endpoint: add --base-url https://your-endpoint/v1
```

**Subscription sidecar** — for people already paying for Codex/ChatGPT or Claude. The sidecar in
`apps/subscription-kit` (also inside every release
archive as `subscription-kit/`) exposes your local, already
logged-in CLI subscription behind an OpenAI-compatible loopback endpoint; no
API key exists anywhere:

```bash
# one-time setup
cd apps/subscription-kit
npm install
npm start                 # serves http://127.0.0.1:8765 and prints a one-time local access token

# in another terminal: use the printed token (it never goes to the model)
export OPENAI_API_KEY='<printed local access token>'
curl -s -H "Authorization: Bearer $OPENAI_API_KEY" \
  http://127.0.0.1:8765/v1/models

# then point translatus at it
translatus translate book.epub --to English \
  --provider openai --base-url http://127.0.0.1:8765/v1 --model <id>
```

In the interactive session (`translatus` with no arguments), pick the
subscription source in Settings and paste the printed token into the
**Access token** row — it is stored in your OS keychain like an API key.

The sidecar rejects every non-health request without that local token. Host
apps may set `LLM_SUB_KIT_TOKEN` themselves instead of using the generated one.
For direct provider API keys, skip the sidecar and use Translatus's normal API
key/keychain path.

Keep `--concurrency` at its default of 1 for subscription backends — they are
heavy and rate-limit-prone under parallel batches. Read the compliance note
in the [FAQ](FAQ.md#can-i-use-my-claude--chatgpt-subscription) first:
the Claude path may stop working at any time and the tool says so. Before using
Codex subscription mode, confirm that you trust the book's source and that its
content contains no malicious instructions. Codex controls its own local-data
access. If you are unsure, use an API key or Ollama.

**Ollama** — free and fully local; the book never leaves your machine:

```bash
ollama pull llama3.3      # once
translatus translate book.epub --to English --provider ollama --model llama3.3
```

Ollama defaults to `http://localhost:11434/v1`; use `--base-url` for a
non-standard port or any other local OpenAI-compatible server (LM Studio,
llama.cpp, vLLM…).

### Translating a book

```bash
translatus translate book.epub --to English [flags]
```

- `--to <label>` — target language, written the way you want the model to
  understand it: `English`, `Français`, `日本語`, `"繁體中文"`…
- `--level standard|expert` — `standard` (the default) is single-pass block
  translation: fast and token-light. (`sentence` is the internal value for
  the same mode and is what `--json` echoes; `fast` is the pre-1.2 alias.
  All are accepted.) `expert` runs the whole-book pipeline:
  a pre-scan builds a glossary and style guide, each chapter translates with
  rolling context, a reflection pass revises the draft against the source,
  and a final pass checks terminology consistency across the book. Slower
  and roughly 2–3× the tokens; worth it for real books.
- `--mode bilingual|replace` — original + translation interleaved (the default), or translation only
  interleaved paragraph by paragraph.
- `--output <path>` — defaults to `<input>.<lang>.<ext>` next to the input.
- `--prompt <text>` — your tone / wording / audience instructions, injected
  into the style section of the translation prompt.
- `--concurrency <n>` — parallel batches for direct API providers. Leave at
  1 for subscription or local backends.

Progress reports per chapter; interrupting at any point is safe (see
[Cache, resume, and free re-export](#cache-resume-and-free-re-export)).

### Margin notes

Margin notes are neutral background notes — history, terminology, cultural
context — written where the book earns them, for the specific reader you
describe. Two entry points:

```bash
# translate and annotate in one pass
translatus translate book.epub --to English --annotate --profile "@me.txt"

# annotate a book you can already read, without translating it
translatus annotate book.epub --profile "First time reading Adam Smith; \
  I know modern economics but nothing about 1770s Britain."
```

- `--profile <text|@file>` — why you are reading, what you hope to get, your
  background. This decides **where** the notes pause and from what angle; the
  notes themselves stay neutral and never address you. Optional when at least
  one `--note-presets` service is picked (picking is often more accurate than
  writing) — but a written line still sharpens the notes.
- `--note-presets terms,history,author,culture,characters,concepts,world,methods,research`
  — the service menu: what the notes should do for you, picked instead of
  written (`world` connects passages to the real world; `methods` unpacks
  methods with their limits; `research` prioritises citable facts, sources and
  structure). With at least one service picked, `--profile` becomes optional.
  Unknown ids warn and are ignored.
- `--note-level beginner|general|insider` — how much the notes assume:
  `beginner` explains in everyday language with examples, `insider` only adds
  what an insider could not easily look up.
- `--density sparse|medium|rich` — how many notes to allow. Sparsity is
  enforced by a hard per-chapter cap in code, not by prompt etiquette.
- `--note-lang <label>` — language the notes are written in. Defaults to the
  translation target language, or the profile's own language when only
  annotating.
- `--note-style <text|@file>` — tone / depth / length instructions, injected
  inside the engine's locked hard rules (which you cannot override: neutral
  background only, no addressing the reader, no reviewing the book).
- `--note-anchors "software engineer,read The Wealth of Nations"` — cognitive
  anchors: what you already know. Notes explain new concepts by bridging from
  this familiar ground (analogies, contrasts) without ever mentioning you.
- `--note-voice study|companion` — the default style register: `study`
  (restrained, the historical default) or `companion` (conversational, more
  short reaction notes). A custom `--note-style` overrides either.
- `--note-profile <file.json|inline JSON>` — the reader-profile contract: one
  document carrying purpose / anchors / presets / voice / lang / density /
  style, fillable by you or by your own AI assistant. Explicit flags override
  document fields. Schema and the standard hand-to-your-AI prompt:
  [READER-PROFILE.md](READER-PROFILE.md).

The pipeline runs four passes: plan (sample the whole book), select (pick
spots per chapter, capped), write (notes for selected passages only), review
(read every note against the rest; drop repetition, fix drift). Contributors
tuning note quality should read [ANNOTATION-TUNING.md](ANNOTATION-TUNING.md).

### Cache, resume, and free re-export

Every translated segment and every note lands in a local SQLite job store
(default `<output>.etjob`, override with `--job`). Consequences:

- **Interruptions cost nothing.** Crash, Ctrl-C, kill, or change your mind —
  re-run the *same command* and cached segments are restored instead of
  re-billed. Only unfinished work is sent to the model.
- **Partial output on demand.** A failed or stopped run still writes the
  book, with untranslated segments keeping the original text. Re-running
  fills only the gaps.
- **Free layout switches.** `--cache-only` re-renders the output from cache
  with zero model calls — switch between `--mode replace` and
  `--mode bilingual` after the fact:

```bash
translatus translate book.epub --to English --cache-only --mode bilingual
```

The cache is keyed by a configuration signature (provider, model, level,
target language, prompt). Changing any of those starts a fresh translation
rather than serving stale text; changing only the note style re-writes notes
while keeping the translation cache intact.

### JSON mode for scripts and agents

`--json` switches stdout to structured events, one JSON document per line,
with a final pretty-printed summary. Nothing human-flavored is printed:

```bash
$ translatus --json translate book.epub --to English | tail -1
```

Every run opens with a `run_start` header (tool version + unix timestamp), so
several runs appended to one log file — the normal interrupt-and-resume
workflow — stay separable afterwards.

Events: `prescan`, `chapter` (index, total, href, units translated/failed),
`annotate_plan`, `annotating`, `notes` (each freshly written note with its
position), `annotate_review` (a payload-free marker that the whole-book
review pass has started; the next event is always `done`), and a closing
`done` object with segment counts, cache hits, token totals, and estimated
cost. Token totals cover the full pipeline (planning, drafting, and review
passes), so they are much larger than the visible note text. Two note counters
with different meanings: `notes_written` is the number of notes actually
present in the finished book; `notes_restored_from_cache` counts cached
per-segment note *decisions* (including "no note here" skips), so it is much
larger and is a resume-progress signal, not a note count. The command is
idempotent and exits non-zero on failure, so the whole thing drops into a
pipeline or an agent loop without wrappers: re-running is always safe.

### Use with your agent (MCP)

`translatus mcp` runs the same engine as a stdio
[Model Context Protocol](https://modelcontextprotocol.io) server, so an agent
can estimate, translate, and annotate books as first-class tools instead of
shelling out. One command registers it with every agent client on the machine:

```bash
translatus mcp install
```

It drives each client's own CLI (`claude mcp add`, `codex mcp add`) instead of
editing their config files, so their formats stay theirs. The interactive
session offers to run it for you the first time you open it — installing the
binary never writes into another program's configuration on its own. Undo with
`translatus mcp uninstall`.

or, in any MCP client that takes a JSON config:

```json
{
  "mcpServers": {
    "translatus": { "command": "translatus", "args": ["mcp"] }
  }
}
```

Three tools are exposed. Parameters use the CLI names without `--`. Results are
an allowlisted metadata subset: fixed status plus counts, booleans, token usage,
and cost — never book/model text or filenames. Three CLI-only parameters are
deliberately absent: API keys never pass through agent context, and agents
cannot choose arbitrary output or cache paths. MCP uses the saved keychain key
or server environment, then derives output and cache names from the input book.

| Tool | Required params | Optional params |
|---|---|---|
| `estimate_book` | `input`, `to` | `level`, `model`, `annotate`, `density` |
| `translate_book` | `input`, `to` | `level`, `provider`, `model`, `base_url`, `prompt`, `mode`, `concurrency`, `cache_only`, `annotate`, `profile`, `note_lang`, `note_style`, `note_presets`, `note_profile`, `note_level`, `note_anchors`, `note_voice`, `density` |
| `annotate_book` | `input` | `profile`, `provider`, `model`, `base_url`, `cache_only`, `note_lang`, `note_style`, `note_presets`, `note_profile`, `note_level`, `note_anchors`, `note_voice`, `density` |

`note_profile` over MCP must be inline JSON (a document starting with `{`);
file paths are refused for the same reason `@file` is. `annotate_book` needs
a reason to pause: `profile`, a `note_profile` carrying `purpose`, or at
least one `note_presets` service.

An example conversation, verbatim:

> **You:** Estimate what translating ~/Books/kokoro.epub to English would
> cost on gpt-4o-mini. If it's under a dollar, translate it with margin
> notes for someone who has never read Meiji-era fiction, and report the
> actual cost.
>
> **Agent:** *(calls `estimate_book`)* About US$0.21 for 9 chapters /
> 1,842 segments, notes included. Proceeding. *(calls `translate_book` with
> `annotate: true`, watches chapter progress)* Done — wrote
> `kokoro.English.epub`, 1,842 segments translated, 41 notes, actual cost
> US$0.24.

Four things to know:

- **Timeouts.** `estimate_book` returns in milliseconds, but a book-length
  `translate_book` runs for minutes to hours. Raise your client's per-tool
  timeout (Claude Code: set `MCP_TOOL_TIMEOUT`, e.g. `3600000` for an
  hour). The server streams MCP `notifications/progress` per chapter, so
  clients that reset their timer on progress can leave defaults generous.
- **Interrupts are still free.** The same job cache applies: if the client
  times out or the run is cut, calling the same tool with the same
  arguments resumes from cache and never re-bills a finished segment.
  There is no separate resume tool because re-calling *is* resuming.
- **One paid book job at a time.** A server keeps estimates, pings, and
  cancellation responsive while one translation or annotation runs, but
  refuses a second paid job until the first finishes or is cancelled. The
  subscription sidecar applies the same default and aborts its provider SDK
  when the HTTP client disconnects.
- **Credentials.** For the provider's normal endpoint, the server uses the
  saved OS-keychain key or provider environment variable. API keys are not a
  tool parameter because that would copy them into agent logs and context. A
  caller-supplied `base_url` receives no ambient key — with one binding: if the
  requested URL is exactly the endpoint saved in your own Settings, the token
  you stored there (OS keychain, not a config file) is sent. That is what makes
  `translatus mcp install` plus a configured sidecar work with no further
  wiring; any other URL an agent names still gets nothing. To bind a different
  endpoint, or to run without saved settings, the operator launching the MCP
  server can pin one explicitly:

  ```bash
  TRANSLATUS_MCP_ENDPOINT_URL=http://127.0.0.1:8765/v1 \
  TRANSLATUS_MCP_ENDPOINT_TOKEN='<sidecar local token>' \
  translatus mcp
  ```

  The scoped token is sent only when the tool call requests that exact URL.
- **Books are untrusted text, and this server keeps them out of your
  agent's context.** A book can contain instructions aimed at whatever
  reads it. Translations are written to a *file*, while progress, successful
  results, and error results carry only fixed status and numeric metadata —
  never book content, filenames, provider output, or subprocess diagnostics.
  Direct API and Ollama requests receive no tool definitions. A
  subscription runtime controls its own access boundary; the Codex trusted-book
  reminder above applies. If you fork this server, keep the return surface metadata-only:
  `mcp_results_never_carry_book_content` is the test that guards it.

### Troubleshooting

- **"no API key found for this provider"** — the fail-fast guard. Export the
  environment variable it names, save the key through the interactive Settings
  screen, or use `--base-url` to
  point at a keyless local endpoint. The error message lists all three setup
  paths.
- **Output looks translated by nobody (mock markers everywhere)** — you ran
  the default `mock` provider. That is the dry run; connect a real model
  (see the note the CLI printed).
- **Interrupted and worried about cost** — do not be; re-run the same
  command. The resume message on Ctrl-C says exactly this.
- **Re-run translated everything again** — the job store did not match. Keep
  the same `--output` (or the same `--job`) and the same provider/model/
  level/prompt flags; any of those changing intentionally invalidates cache.
- **Sidecar refuses connections** — is `npm start` still running? The kit
  binds to `127.0.0.1:8765` and only accepts loopback requests. If that port
  is taken by something else, start it on another one:
  `LLM_SUB_KIT_PORT=8790 npm start` — or skip the hand-run sidecar entirely
  with `--provider subscription`, which picks its own free port.
- **"Native CLI binary … not found" mid-run** — the sidecar's install was
  changed underneath it (typically a package upgrade removing the old
  directory while a translation was running). Restart the run: cached
  segments are never re-billed. `translatus doctor` detects this state.
- **Not sure what state the install is in** — run `translatus doctor`. It
  checks binary/PATH self-consistency, the sidecar kit and Node, the default
  sidecar port, per-client MCP registration, and the settings file.
- **Ollama found but model missing** — `ollama pull <model>` first; then
  pass the exact model name to `--model`.
- **A chapter keeps failing** — the summary reports failed segments and the
  output keeps their original text. Re-running fills only those. If it
  persists, the model is likely refusing that passage; try `--level expert`
  or a different model, and open an issue with the (non-sensitive) details.

### Common flags

| Flag | Commands | Meaning |
|---|---|---|
| `--to <label>` | translate, estimate | target language label |
| `--level standard\|expert` | translate, estimate | pipeline depth (`sentence`/`fast` = aliases of `standard`) |
| `--mode replace\|bilingual` | translate | output layout |
| `--provider mock\|openai\|ollama` | translate, annotate | model backend |
| `--model <name>` | all | model id |
| `--base-url <url>` | translate, annotate | OpenAI-compatible endpoint |
| `--output <path>` | translate, annotate | output file |
| `--job <path>` | translate, annotate | cache/resume store |
| `--cache-only` | translate, annotate | re-render, zero model calls |
| `--annotate` | translate, estimate | add margin notes |
| `--profile <text\|@file>` | translate, annotate | reader background |
| `--note-presets <ids>` | translate, annotate | preset help angles |
| `--note-lang <label>` | translate, annotate | note language |
| `--note-style <text\|@file>` | translate, annotate | note tone/depth/length |
| `--density sparse\|medium\|rich` | translate, annotate, estimate | note volume |
| `--concurrency <n>` | translate | parallel batches |
| `--prompt <text>` | translate | translation style instructions |
| `--json` | all | machine-readable output |
