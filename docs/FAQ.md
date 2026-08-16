# FAQ

- [How much does translating a book cost?](#how-much-does-translating-a-book-cost)
- [Can I use my Claude / ChatGPT subscription?](#can-i-use-my-claude--chatgpt-subscription)
- [Where does my data go?](#where-does-my-data-go)
- [How is this different from bilingual_book_maker?](#how-is-this-different-from-bilingual_book_maker)
- [Will I be billed twice if I stop and restart?](#will-i-be-billed-twice-if-i-stop-and-restart)
- [Does it support PDF?](#does-it-support-pdf)
- [Which model should I use?](#which-model-should-i-use)
- [Are the margin notes going to spoil or editorialize the book?](#are-the-margin-notes-going-to-spoil-or-editorialize-the-book)
- [Is translating a book I bought even legal?](#is-translating-a-book-i-bought-even-legal)
- [How is Translatus licensed?](#how-is-translatus-licensed)

### How much does translating a book cost?

Whatever your model charges — Translatus adds nothing. As a shape: a 300-page
novel is roughly 120k–180k tokens of input and slightly more output. On a
small hosted model (gpt-4o-mini class) that is well under a dollar in standard
mode; on a frontier model, a few dollars. Expert mode costs roughly 2–3×
standard mode; margin notes add a fraction on top (the estimate shows it
separately). Ollama is free. Run `translatus estimate book.epub --to <lang>
--model <model>` before anything — human-mode runs also print the estimate
automatically before starting.

### Can I use my Claude / ChatGPT subscription?

As of mid-2026:

- **Codex (ChatGPT) subscription**: supported. Before using it, confirm that
  you trust the book's source and that its content contains no malicious
  instructions. Codex controls its own local-data access. If you are unsure,
  use an API key or Ollama.
- **Claude subscription**: Anthropic's terms restrict third-party routing of
  Pro/Max subscription credentials, and Agent SDK usage is metered against a
  small per-plan credit. This mode may stop working at any time, and both
  the sidecar says so in its UI. We do not recommend building
  your workflow on it.
- **The stable paths are an API key or Ollama.** The Claude sidecar is optional and
  local; your credentials never pass through anything of ours.

### Where does my data go?

To exactly one place: the model endpoint you configured, in batches, during
a run. There is no telemetry, no analytics, no account, no update ping in
the engine or CLI. API keys are resolved from an explicit flag, the OS
keychain, or the provider environment variable. Prefer the keychain or
environment: command-line values can appear in shell history and process
listings. Keys are never written to a settings file.
With Ollama, nothing leaves your machine at all.
[SECURITY.md](../SECURITY.md) documents the full threat model — including
how to verify the no-telemetry claim yourself instead of trusting us.

### How is this different from bilingual_book_maker?

[bilingual_book_maker](https://github.com/yihong0618/bilingual_book_maker)
pioneered this category and remains a fine one-command tool. Translatus
differs where book-scale translation hurts:

- **Byte-faithful structure.** The engine rewrites only text nodes in a
  mini-DOM; tags, entities, CSS, and package structure survive byte-for-byte.
- **Terminology enforcement.** Expert mode locks a glossary and enforces it
  by sentinel substitution in the text, not by trusting the model's memory.
- **A resumable, content-addressed cache.** Interrupt anywhere; re-runs
  never re-bill; layout switches re-export for free.
- **Margin notes.** Reader-aware background annotation with a whole-book
  review pass — no equivalent exists in the category.

If you just want a quick bilingual EPUB with minimal setup, bbm is simpler.

### Will I be billed twice if I stop and restart?

No. Every translated segment is cached in the local job store the moment it
lands. Ctrl-C, crashes, and week-later re-runs all resume from cache; only
unfinished segments are sent to the model. This is an engine invariant with
regression tests, not a best effort.

### I installed it — why doesn't my agent's MCP list show translatus?

Installing puts exactly one binary on your machine. The MCP server is a
capability of that binary (`translatus mcp`), not a separate install, and
nothing registers itself into your agent's configuration behind your back —
that registration lives in your agent's own config and is yours to make.
One line for Claude Code:

```bash
claude mcp add translatus -- translatus mcp
```

Any other MCP client: point it at command `translatus`, args `["mcp"]`.
Agents with shell access don't even need this — the CLI's `--json` mode is
built for them.

### Does it support PDF?

Not yet. EPUB and TXT are the supported formats. PDF is the most-requested
addition. Status: not started, and hard to do well (PDF is
a layout format, not a text format). If your book exists as EPUB, use that.

### Which model should I use?

Whatever you have access to; there is no house model. Rules of thumb from
our evaluations: standard mode with a small hosted model is fine for casual
reading of contemporary prose; literary or terminology-heavy books benefit
from expert mode with a frontier model; margin notes are only as good as the
model you bring, since they require actual background knowledge. The `mock`
provider exists so you can verify formatting fidelity for free before
choosing.

### Are the margin notes going to spoil or editorialize the book?

The engine's hard rules — baked into the prompt and not overridable by user
style — forbid addressing the reader, reviewing the book, and revealing
later plot. Notes are neutral background (history, terminology, context),
visibly marked as notes, placed before a passage to set the stage or after
it to go deeper. Sparsity is enforced by a per-chapter cap in code. Your
reader profile decides where notes appear and from what angle — not what
opinions they hold, because they hold none.

### Is translating a book I bought even legal?

Translatus runs on your machine with your model account; nothing is uploaded
to us and nothing is redistributed by the tool. Translating a book you own
for your own reading is personal use in most jurisdictions, like making
personal notes in the margins. Distributing the output is a different matter
entirely — that is between you and copyright law, and this tool does not
help you do it.

### How is Translatus licensed?

Engine and CLI: MIT. Third-party licenses are listed in
[THIRD-PARTY-LICENSES.md](../THIRD-PARTY-LICENSES.md), and the projects that
shaped the design are credited in
[ACKNOWLEDGMENTS.md](ACKNOWLEDGMENTS.md).
