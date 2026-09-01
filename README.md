<p align="center">
  <img src="docs/logo.png" width="128" alt="Translatus" />
</p>

<h1 align="center">Translatus</h1>

<p align="center">An open-source CLI that translates and annotates whole books with your own LLM, locally.</p>

<p align="center">
  <a href="https://doka.software/translatus"><img src="https://img.shields.io/badge/website-doka.software-4a4a4a.svg" alt="Website"></a>
  <a href="https://github.com/doka-software/translatus/actions/workflows/ci.yml"><img src="https://github.com/doka-software/translatus/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="MIT"></a>
  <a href="README.zh-TW.md"><img src="https://img.shields.io/badge/說明-繁體中文-b23a2a.svg" alt="繁體中文"></a>
</p>

## Why Translatus

A book is fixed the day it is printed. Its readers are not. Language, missing
context, and distance in time keep people out of good books. Translatus uses
your own model to translate the whole book and write margin notes where you
need them, turning a fixed book into an edition prepared for you.

Pasting chapters into a chatbot loses formatting, forgets terminology, and
dies halfway through the book. Translatus treats the whole book as the unit of
work:

- **The output is a real book.** Only text nodes change. Inline tags,
  attributes, entities, CSS, images and the EPUB structure survive
  byte-for-byte. The result opens clean in any reader.
- **Terminology holds across 400 pages.** Expert mode scans the book first,
  locks a glossary, translates each chapter with rolling context, then
  revises the draft against the source. Locked terms are enforced by
  sentinel substitution, not by trusting the model.
- **Interruptions cost nothing.** Every segment is cached in a local SQLite
  store. Crash, pause, or change your mind next week. Re-running never
  re-bills a translated segment, and switching layouts (translated ↔
  bilingual) re-exports from cache with zero LLM calls.
- **Margin notes, written for you.** Pick what the notes should do (nine
  services from term explanations to real-world connections), set how much
  they assume (`beginner` to `insider`), choose a quiet or a spoken voice,
  and optionally tell it who you are; your own AI assistant can fill that
  reader profile for you ([schema + hand-to-your-AI prompt](docs/READER-PROFILE.md)).
  The engine maps the whole book first, annotates only the passages that earn
  it (before a paragraph to set the stage, or after it to reach back to what
  you have read, never ahead), then reviews every note against the rest.
  Notes stay neutral background, clearly marked, never blended into the
  author's text, and are program-checked to never address you.
- **Your model, your choice.** Use a local Codex/ChatGPT or Claude Code login,
  any OpenAI-compatible API key, or a local Ollama model. Book text goes only
  to the endpoint you select; keys live in the OS keychain.
- **Built for agents too.** The CLI speaks structured JSON, is idempotent,
  and resumes safely. It also ships as an MCP server: `translatus mcp` lets
  Claude Code or any MCP client estimate, translate, and annotate books
  directly ([see below](#use-with-your-agent-mcp)).

## Why you might not want it

- **Your book is a PDF.** Only EPUB and TXT are supported. PDF is the
  most-requested format and it is not started.
- **You want one quick bilingual file with the least possible setup.**
  [bilingual_book_maker](https://github.com/yihong0618/bilingual_book_maker)
  is simpler for that; Translatus earns its keep at whole-book scale.
- **You expect the tool to bring a model.** It never does. Quality is
  exactly the model you connect, and a weak local model produces weak
  translations and shallow notes.

## Quick start

```bash
brew install doka-software/tap/translatus     # Homebrew (builds in about a minute)
```

Or grab a prebuilt binary for macOS (Apple silicon or Intel) from the
[Releases page](https://github.com/doka-software/translatus/releases) and
drop it on your `PATH`, or build from source with Rust (stable):

```bash
cargo install --locked --path apps/cli        # installs `translatus`
```

Run it with no arguments and it opens an interactive session:

```bash
translatus
```

It finds the books around you and offers every control the flags do.
Translation and margin notes are two services you switch on independently,
so leaving notes on with translation off annotates a book without touching its
text. Per book you choose the target language, quick or expert depth, the
layout, and for notes: who you are and why you are reading, which kinds of help
you want, how dense the notes should be, and what language they are written in.
A Settings screen holds the model source (a subscription, your own API key, or a
local Ollama model), the key itself (kept in your OS keychain, never in a config
file), a base URL, and a connection test.

The session speaks English, 繁體中文, 简体中文, 日本語, and 한국어. It
follows your terminal locale, or set `TRANSLATUS_LANG=zh_TW` (etc.) to
override. Settings persist locally in a `settings.json`. Point
`TRANSLATUS_CONFIG_DIR` somewhere else if you would rather keep a separate
profile.

Nothing runs until you agree to a quantified summary of the work and the cost,
Esc always steps back exactly one screen, and everything the session can do is
also a flag: it prints the equivalent command as you go, so you can graduate to
the CLI whenever you like. In a pipe or a script, a bare `translatus` prints
usage instead; the interactive session only opens on a terminal.

```bash
# What would this cost? (no translation happens)
translatus estimate book.epub --to English

# Translate a book
translatus translate book.epub --to English --output book.en.epub

# Expert mode: whole-book consistency, slower, worth it for real books
translatus translate book.epub --to English --level expert

# Margin notes on a book you can already read
translatus annotate book.epub --profile "@my-background.txt"

# Pick services without typing a profile (terms / history / real-world links...)
translatus annotate book.epub --note-presets terms,world --note-level beginner

# A spoken voice, bridging from what you already know
translatus annotate book.epub --note-presets culture,world --note-voice companion \
  --note-anchors "software engineer,ran a small team"

# Or let YOUR OWN AI fill the whole reader profile (docs/READER-PROFILE.md)
translatus annotate book.epub --note-profile profile.json

# Translate and annotate in one pass
translatus translate book.epub --to English --annotate --profile "..."

# Re-export a finished book in a new layout: free, no LLM calls
translatus translate book.epub --to English --cache-only --mode bilingual

# JSON in, JSON out, resume-safe: for scripts and agents
translatus --json translate book.epub --to English
```

The target is any label the model understands: `English`, `Français`,
`日本語`, `"繁體中文"`… Interrupted anything? Run the same command again.
Cached segments are never re-billed.

Providers: `--provider openai|ollama|mock` with `--model`. The
`mock` provider is a flag-only dry run for scripts and tests: it runs the
full pipeline offline for free to verify format fidelity before spending
tokens; the interactive session only offers real model sources. Human-mode runs print a cost
estimate before starting, and `translatus --help` ends with a provider
setup card (including the no-API-key subscription sidecar).

## Uninstall & diagnostics

Something not working? `translatus doctor` checks the binary, the
subscription sidecar, the sidecar port, and every agent's MCP registration in
one shot, and tells you exactly what to fix.

To remove Translatus:

1. Remove the binary: `brew uninstall translatus` (Homebrew), delete it from
   wherever you placed it on `PATH` (prebuilt download), or `cargo uninstall
   translatus` (source install).
2. Remove MCP registrations first, while the binary is still on `PATH`:
   `translatus mcp uninstall` (or `--client claude` / `--client codex` for
   just one).
3. Delete local settings: `rm -rf "$HOME/Library/Application Support/translatus"`
   (or wherever `TRANSLATUS_CONFIG_DIR` points).
4. API keys live in the OS keychain, not in a file: open Keychain Access,
   search "translatus", and delete any entries it finds.

Nothing about your books or their output is touched; those stay wherever you
chose to keep them.

## Use with your agent (MCP)

`translatus mcp` runs a stdio [MCP](https://modelcontextprotocol.io) server
exposing three tools: `estimate_book`, `translate_book`, and
`annotate_book`, with per-chapter progress notifications. Tool results expose
only fixed status and numeric metadata; translated text and filenames stay out
of the calling agent's context. One line for Claude Code:

```bash
translatus mcp install     # registers with every agent client it finds
```

It goes through each client's own CLI (`claude mcp add`, `codex mcp add`) rather
than editing their config files. The first time you open the interactive session
it offers to do this for you, once. Installing the binary never writes into
another program's configuration behind your back. Undo with
`translatus mcp uninstall`, which only removes registrations that
`mcp install` itself created; target one client explicitly with
`--client claude` / `--client codex`. `translatus doctor` reports the
install's health (binary, sidecar, port, registrations) in one shot (see
[Uninstall & diagnostics](#uninstall--diagnostics)).

Or in any MCP client's JSON config:

```json
{
  "mcpServers": {
    "translatus": { "command": "translatus", "args": ["mcp"] }
  }
}
```

Then ask your agent, in plain language:

> Estimate what translating ~/Books/kokoro.epub to English would cost on
> gpt-4o-mini, then translate it with margin notes for someone who has
> never read Meiji-era fiction, and tell me the actual cost when it's done.

Book-length calls run for minutes; set your client's tool timeout
accordingly (details and per-tool parameters in the
[user guide](docs/GUIDE.md#use-with-your-agent-mcp)).

## Documentation

- **[User guide](docs/GUIDE.md)**: install, connect a model (API key /
  subscription sidecar / Ollama), every flag with examples, cache and
  resume behavior, JSON mode, MCP server, troubleshooting.
- **[FAQ](docs/FAQ.md)**: costs, subscription compliance, why the engine
  is free, where your data goes, how this differs from prior art.
- **[Annotation tuning map](docs/ANNOTATION-TUNING.md)**: for contributors
  changing note quality.
- **[CONTRIBUTING.md](CONTRIBUTING.md)** · **[SECURITY.md](SECURITY.md)** ·
  **[CHANGELOG.md](CHANGELOG.md)**

## What the notes will not do

Margin notes follow hard rules baked into the engine prompt, not etiquette:

- Neutral background only: history, context, terminology, structure.
- Never address the reader or claim what a passage "means to you".
- Never review the book ("this famous passage…").
- Sparse by architecture: the engine picks annotation spots per chapter
  under a hard cap. Most paragraphs stay quiet.

Your reader profile decides **where** it pauses and **from what angle**. The
connecting is left to you.

## Fidelity and privacy design

- Original text is byte-faithful in the output; translations and notes are
  separate, labelled blocks.
- Everything runs locally: parsing, caching, reassembly. Book text is sent
  only to the model endpoint you configured, in batches.
- No telemetry. No account. API keys go to the OS keychain, not to files.

## Building from source

```bash
cargo build && cargo test          # engine + CLI
```

The optional subscription sidecar (`apps/subscription-kit`, Node ≥ 20; also
shipped inside every release archive as `subscription-kit/`) exposes your local
Codex/ChatGPT or Claude Code login behind an OpenAI-compatible endpoint:

```bash
cd apps/subscription-kit && npm install && npm run smoke
npm start  # prints the one-time local access token required by clients
```

If you choose Codex subscription mode, first make sure you trust the book's
source and that its content contains no malicious instructions. Codex controls
its own local-data access. If you are unsure, use an API key or Ollama. Claude
subscription users should also read the policy note shown by the sidecar.

## Architecture

```
crates/core/   the translation + annotation engine (no UI concepts)
  format/      byte-faithful XHTML mini-DOM · EPUB · TXT · placeholders
  translate/   standard + expert passes · prompts · glossary enforcement
  annotate/    chapter-level spot selection · notes · whole-book review
  job.rs       SQLite cache + checkpoints (resume without re-billing)
apps/cli/      thin wrapper: JSON I/O, idempotent, MCP server
apps/subscription-kit/   optional local subscription sidecar
```

## Support

The engine and CLI are MIT-licensed and free forever, with nothing held back
for a paid tier. If Translatus carried a book across for you, here is how to
help it keep going:

- **Contribute:** star [doka-software/translatus](https://github.com/doka-software/translatus),
  report a bug, or send a PR. Every size counts.
- **Sponsor:** [Ko-fi](https://ko-fi.com/dokasoftware) <!-- SPONSOR-LINKS -->

Maintained by [@doka-doka](https://github.com/doka-doka).

## Prior art & acknowledgements

Translatus stands on ideas proven by earlier projects. Full notes in
[docs/ACKNOWLEDGMENTS.md](docs/ACKNOWLEDGMENTS.md); the short version:

- [bilingual_book_maker](https://github.com/yihong0618/bilingual_book_maker)
  (MIT) pioneered the category: one-command whole-EPUB bilingual output with
  your own API key; Translatus differs in byte-faithful XHTML handling and a
  resumable, content-addressed cache.
- [Ebook-Translator Calibre plugin](https://github.com/bookfere/Ebook-Translator-Calibre-Plugin)
  (GPLv3) showed how far position modes and cache-only re-rendering can go;
  we studied its behavior only and reimplemented everything clean-room in
  Rust: no code was copied, GPLv3 respected.
- [translation-agent](https://github.com/andrewyng/translation-agent)
  (Andrew Ng) demonstrated the translate → reflect → improve loop that shaped
  our expert mode's source-aware reflection pass.
- [DelTA](https://arxiv.org/abs/2410.08143) (ICLR 2025) made the case for
  document-level translation as structured multi-level memory rather than a
  bigger context window. It is the blueprint behind our whole-book
  consistency passes.

## License

Engine and CLI: [MIT](LICENSE) © doka.software and Translatus contributors.
