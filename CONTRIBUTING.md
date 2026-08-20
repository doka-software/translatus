# Contributing to Translatus

Thanks for helping ferry books across languages! This document covers how to
build, test, and submit changes.

## Building

Prerequisites: Rust (stable) and Node.js ≥ 20.

```bash
# Engine + CLI
cargo build
cargo test

# Optional subscription sidecar
cd apps/subscription-kit && npm install
```


## Testing

- `cargo test --workspace` must pass.
- End-to-end format-fidelity checks use the built-in `mock` provider (free,
  offline): `cargo run -p translatus -- translate book.epub --to "繁體中文"`.
- Subscription sidecar: `cd apps/subscription-kit && npm run smoke`
  (offline by default; `RUN_LIVE=1` adds real probes and spends tokens).

## Capability parity (CAPABILITIES.toml)

Every user-visible capability that goes through the core engine (a
`TranslateConfig` field or a CLI command/flag) is declared once in
[`CAPABILITIES.toml`](CAPABILITIES.toml). **Adding or changing a capability
means updating that manifest in the same PR** — the parity tests
(`apps/cli/src/parity_tests.rs`, run via `cargo test -p translatus`) enforce it:

- a new `TranslateConfig` field that no capability claims turns CI red;
- a declared command/flag that doesn't exist in the CLI turns CI red;
- every capability must anchor at least one real test function.

Deliberate asymmetry (CLI-only or GUI-only capabilities) is allowed but must
carry a written `reason`. See the manifest header for the schema and the
inclusion criteria.

## Code style

- Rust: `cargo fmt --all` before committing; `cargo clippy --workspace
  --all-targets` must be warning-free. CI enforces both.
- Engine invariants to respect: byte-faithful output, keys only in the OS
  keychain, cache never re-bills, cancellation must be immediate.

## Pull requests

- Keep PRs focused; separate refactors from behavior changes.
- Write commit messages in English: a short imperative subject line, plus a
  body explaining *why* when it isn't obvious.
- Include tests for engine changes (DOM round-trip, placeholder, alignment,
  and cache behaviors all have existing test patterns to follow).
- If you touch `apps/subscription-kit/`, note that it is **vendored** from
  the upstream `llm-subscription-kit` repo — see
  [`apps/subscription-kit/VENDORED.md`](apps/subscription-kit/VENDORED.md).
  Fix upstream first whenever possible.

## Reporting issues

Use the issue templates. For security problems, see [SECURITY.md](SECURITY.md)
instead of opening a public issue.
