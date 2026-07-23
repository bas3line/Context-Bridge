# Changelog

All notable changes are documented here. This project follows Semantic
Versioning and the Keep a Changelog structure.

## [Unreleased]

### Planned

- Guarded production OpenCode structured export/server profile
- Guarded Claude Code resume and opt-in hook profile
- Guarded Codex discovery/import and native resume profile

## [0.1.0] - 2026-07-24

### Added

- Nine-crate Rust workspace with vendor-neutral domain boundaries
- Append-only SQLite event log with embedded migrations, WAL, foreign keys,
  deduplication, monotonic sequences, and restrictive permissions
- Canonical project identity plus Git/filesystem before-and-after snapshots
- Typed events, external links, checkpoints, and handoff packages
- Deterministic reducer, approximate token budgeting, and target renderers
- Strict local secret classification, redaction, and path exclusions
- Attached interactive process launching without terminal transcript scraping
- CLI run, continue, import, sessions, show, timeline, diff, checkpoint, export,
  doctor, integration guard, and configuration commands
- Fake JSONL compatibility profile and complete Claude → OpenCode → Codex test
- Versioned, malformed, unknown-field, redaction, crash, and golden fixtures

[Unreleased]: https://github.com/context-bridge/context-bridge/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/context-bridge/context-bridge/releases/tag/v0.1.0
