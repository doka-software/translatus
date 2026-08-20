# Agent guide

You are probably an AI assistant helping someone use or change Translatus.
This page tells you where everything is, so you don't have to discover it
by trial and error.

## Using the tool (not changing it)

- `translatus --help` and each subcommand's `--help` are complete and current.
- Prefer `--json`: structured events, one `done` object at the end, and
  re-running the same command resumes from cache at zero cost.
- Or register the MCP server: `translatus mcp install` (registers with the
  agent clients found on the machine; `translatus mcp` alone is what they then
  launch). Tools: `estimate_book`, `translate_book`, `annotate_book`; schemas
  carry every parameter.
- The reader profile your user fills (or you fill for them):
  [docs/READER-PROFILE.md](docs/READER-PROFILE.md). Over MCP, pass
  `note_profile` as inline JSON only — file paths are refused by design.
- Always run `estimate_book` / `translatus estimate` before a paid run.

## Changing the code

Read these in order; they are maps, not tutorials:

1. **[CAPABILITIES.toml](CAPABILITIES.toml)** — the machine-readable feature
   manifest. Every user-visible capability declares its config fields, CLI
   flags, and test anchors. Parity tests enforce it in both directions:
   adding a `TranslateConfig` field without declaring a capability is a red
   test, and so is declaring a flag that doesn't exist. When you add or
   change a feature, this file changes in the same commit.
2. **[docs/ANNOTATION-TUNING.md](docs/ANNOTATION-TUNING.md)** — margin-note
   quality knobs as a "change X → edit here" table (prompts, caps, machine
   checks, cache-signature rules). Start here for any note-quality work.
3. **Module docs** — `crates/core/src/annotate/mod.rs` (the four-pass
   annotation pipeline) and `crates/core/src/translate/` carry the
   authoritative pipeline descriptions as top-of-file comments.
4. **[CONTRIBUTING.md](CONTRIBUTING.md)** — build, test, style, PR rules.

Layout: `crates/core` is the engine (all logic); `apps/cli` is a thin shell
(CLI + TUI + MCP server over the same engine); `apps/subscription-kit` is
the optional Node sidecar for subscription-based model access.

## Invariants you must not break

- **Byte-faithful output**: only text nodes change; markup, styles and
  images pass through untouched. Tests enforce this.
- **Cache signatures are billing contracts**: `cache_signature()` /
  `annotation_signature()` decide when users get re-billed. Changing what
  enters a hash, or the salt, is a product decision — read the comments in
  `crates/core/src/config.rs` first, and never "fix" a golden signature test
  just to make it green.
- **Notes never address the reader, never spoil ahead** — hard rules in
  `crates/core/src/annotate/prompt.rs`, enforced by program-side checks
  (`note_addresses_reader`, `note_opens_generic`), not by prompt hope.
- **MCP results never carry book content** back to the calling agent
  (`mcp_results_never_carry_book_content`): tool results reach a privileged
  context, so book text crossing back is prompt injection, not noise.
- **No telemetry.** The engine makes no network calls except to the model
  endpoint the user configured. Keep it that way.

## Verify before you claim done

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

All three green, plus a `CAPABILITIES.toml` entry for anything user-visible,
is the bar for a PR.
