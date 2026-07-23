# Architecture

Context Bridge separates vendor parsing, canonical facts, persistence, project
inspection, and target rendering:

```text
Agent session
    ↓
Capability-gated adapter
    ↓
Versioned raw events
    ↓
Normalization and secret classification
    ↓
Append-only canonical event log (SQLite)
    ↓
Deterministic reducer and budget compactor
    ↓
Canonical handoff package
    ↓
Target renderer
    ↓
Target agent session
```

## Package boundaries

- `cb-core`: strongly typed domain models, repository ports, clock/scanner
  ports, and adapter contracts. It has no SQLite, process, or vendor-format
  dependency.
- `cb-storage`: SQLite implementation with embedded migrations, WAL, foreign
  keys, monotonic per-session sequences, unique import keys, and transactions.
- `cb-project`: canonical project identity, Git inspection, filesystem metadata
  snapshots, and before/after change reconciliation. It never runs destructive
  Git commands.
- `cb-security`: default path exclusions, sensitivity classification, and
  deterministic line redaction.
- `cb-context`: raw-event normalization, deterministic reduction, token
  estimation, compaction, and target prompt rendering.
- `cb-process`: attached child execution using the caller's controlling
  terminal. It does not capture or interpret terminal streams.
- `cb-adapters`: detection and compatibility profiles. Vendor details remain
  outside the domain layer.
- `cb-cli`: configuration layering, diagnostics, command orchestration, and
  human/JSON output.
- `cb-test-support`: temporary Git projects and fake interactive executables.

## Source of truth and derived state

`context_events` is append-only. Each import has a stable import key; duplicate
refreshes are ignored by a database uniqueness constraint. Sequence numbers are
allocated monotonically inside the import transaction.

Checkpoints provide named positions in the event stream. Handoff packages are
stored with their schema version and content hash, but can always be rebuilt
from events plus current project state. A target renderer may change headings
or agent-specific framing, but not the facts in the package.

## Run lifecycle

1. Resolve a canonical Git root, or canonicalize the requested directory.
2. Upsert project identity and create an active bridge session.
3. Capture initial Git and filesystem snapshots.
4. Launch the adapter with inherited terminal streams.
5. For an adapter with a supported structured protocol, import its records.
6. Capture a second snapshot and derive file/Git events.
7. Link an external session ID only when the adapter can prove it.
8. Write a checkpoint and derived summary.
9. Preserve the child's exit status.

Production adapters are guarded by a detected, documented CLI profile. OpenCode
1.18 discovers sessions and imports sanitized JSON exports; Claude Code 2.1 and
Codex CLI 0.145 use their documented prompt/resume commands and link only IDs
they can prove. No production adapter parses terminal output or reads
vendor-private storage. The Fake JSONL v1 profile remains test-only and
exercises fixture-driven structured import.

The child already owns the same controlling PTY as Context Bridge, so colors,
terminal resize, foreground signals, and terminal modes flow through the OS.
Context Bridge does not enter raw mode itself, which means there is no terminal
state for it to restore after a crash.

## Continue lifecycle

`continue` resolves one canonical session, refreshes the source link when its
profile supports it, captures current project state, builds a redacted package,
applies the requested budget, and renders a target bootstrap. A bridge-created
target link is resumed when native resume is supported; otherwise a new target
session is created and linked to the same bridge session.

The repository is explicitly authoritative over stale handoff text.

## Failure behavior

- Malformed external records are skipped when other records remain usable and
  are reported through structured diagnostics.
- An entirely malformed import fails before bridge-session creation.
- Event imports are transactional; foreign-key or uniqueness failures do not
  leave partial event batches.
- An agent's non-zero exit still triggers project reconciliation and database
  closure, then becomes the `cb` exit status.
- Unsupported real-agent profiles never fall through to an undocumented
  parser or guessed prompt flag.
