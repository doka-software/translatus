## What & why

<!-- What does this change, and what problem does it solve? -->

## How it was tested

<!-- cargo test? mock-provider end-to-end? -->

## Checklist

- [ ] `cargo fmt --all` and `cargo clippy --workspace --all-targets` are clean
- [ ] `cargo test --workspace` passes
- [ ] User-visible capability changes are declared in `CAPABILITIES.toml` (the parity tests enforce this)
- [ ] Changes to `apps/subscription-kit/` were made upstream first (see its `VENDORED.md`)
