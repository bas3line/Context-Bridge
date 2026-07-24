# Contributing

Thank you for improving Context Bridge.

## Development setup

```bash
git clone https://github.com/bas3line/Context-Bridge
cd context-bridge
cargo test --workspace --all-features
```

The pinned toolchain includes `rustfmt` and Clippy. Install `cargo-deny`
separately for dependency policy checks.

## Required gates

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo deny check
```

Add unit tests for domain behavior and fixture/golden coverage for every parser
or renderer change. Agent compatibility changes must include at least two
anonymized supported fixture versions and malformed input.

## Design rules

- Keep vendor formats in `cb-adapters`.
- Keep SQLite types in `cb-storage`.
- Preserve the append-only event log as the source of truth.
- Do not parse ANSI terminal output when a structured source exists.
- Do not modify external session files.
- Do not add telemetry or network activity by default.
- Never add a known credential to a fixture, commit, issue, or test log.
- Do not enable unknown versions under a parser profile known to be unsafe.

## Pull requests

Describe the user-visible behavior, compatibility evidence, privacy impact, and
commands used for validation. Keep unrelated changes separate.
