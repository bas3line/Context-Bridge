# Context Bridge

<p align="center">
  <strong>Move coding work between AI agents without losing the plot.</strong><br>
  A local-first CLI for durable, redacted context handoffs.
</p>

<p align="center">
  <code>local-only</code> · <code>redaction-aware</code> · <code>evidence-based</code> · <code>vendor-neutral</code>
</p>

```text
 OpenCode ── import / run ──┐
                            │  canonical event log
 Claude Code ── continue ───┼──► reviewed, redacted handoff
                            │
 Codex ──────── continue ───┘
```

Most agent CLIs make it easy to start a session and awkward to leave one.
**Context Bridge** (`cb`) records the observable work around a coding session,
keeps it on your machine, and creates a focused continuation for the next
agent. It transfers useful evidence—not an imaginary perfect clone of hidden
model state, private reasoning, or provider-side caches.

> **Status:** early `0.1.0`. The canonical event model, local storage, and
> redaction boundary are ready to evaluate. Agent adapters are deliberately
> narrow, documented, and version-gated.

## Why use it?

Switch models because the task changed—not because starting over is cheaper
than preserving the thread.

| Without a bridge | With Context Bridge |
| --- | --- |
| Re-explain the task and repository state | Start with the current objective, Git state, decisions, tests, and next action |
| Paste logs into a giant prompt | Keep an ordered, local canonical event log and generate a bounded handoff |
| Hope a vendor database format is stable | Use documented adapter protocols with explicit compatibility checks |
| Risk copying secrets into the next session | Classify sensitive events and exclude known credential paths before handoff |

## Quick start

Install from a checkout, then inspect the agent profiles available on this
machine:

```bash
cargo install --path crates/cb-cli --locked
cb doctor --verbose
```

### Start fresh, import history, or resume

For OpenCode, `cb run opencode` opens a small session deck. Pick a fresh run,
import an existing session, resume a bridged one, or inspect local sessions.

```bash
cb run opencode
```

To move an existing supported OpenCode session into Claude Code:

```bash
# Reads the documented sanitized export. The source session is never changed.
cb import opencode --session <opencode-session-id>

# Inspect exactly what the next agent will receive.
cb continue --session <bridge-session-id> --to claude --preview

# Launch Claude Code with the handoff.
cb continue --session <bridge-session-id> --to claude
```

Need raw local text that OpenCode's sanitizer omits? Opt in consciously:

```bash
cb import opencode --session <opencode-session-id> --full
```

`--full` can include sensitive text. Review and keep that bridge data local.
See the [session import guide](docs/import.md) for the complete workflow,
recency-first context budgets, and continuation behavior.

## What moves—and what does not

Context Bridge preserves observable, useful work:

- User, assistant, and system messages from supported imports
- Structured tool calls, relevant results, commands, tests, and errors
- Files inspected or changed, plus Git branch, HEAD, status, and diffs
- Decisions, assumptions, checkpoints, and the recommended next action
- Provenance, content hashes, parser versions, and source event identifiers

It intentionally does **not** claim to transfer hidden model state,
chain-of-thought, provider-side KV caches, vendor-owned session records, or
undocumented private databases. There is no Context Bridge cloud service, raw
terminal capture, telemetry, or source-session mutation.

The canonical append-only event log stays on the operator's machine. Handoffs
are derived from it, so they can be reviewed or regenerated later.

## How a handoff works

```text
1. Capture     Start through cb, or import a documented external export.
2. Reconcile   Record project/Git state, checkpoints, and observable events.
3. Reduce      Build a deterministic package within the target context budget.
4. Protect     Apply path exclusions and secret classification before injection.
5. Continue    Preview or launch the target through its documented interface.
```

Large sessions remain local in full. A target prompt still has a finite context
window, so `--budget` controls the generated handoff—not the amount of history
stored. Conversation selection is **recency-first**: Context Bridge keeps the
newest supported conversation that fits alongside the current objective and
structured repository state.

```bash
cb continue --session <bridge-session-id> --to claude --budget 100000 --preview
```

For Claude Code, the handoff is passed through its documented
`--append-system-prompt-file` interface, avoiding shell argument-length limits.

## Compatibility

Adapters take the safe path when a tool's documented protocol is unavailable:
they refuse structured import rather than guessing flags or reading private
agent data.

| Capability | Codex CLI | Claude Code | OpenCode |
| --- | --- | --- | --- |
| Guarded version detection | 0.145 | 2.1 | 1.18 |
| Interactive launch | Yes | Yes | Yes |
| Historic structured import | Not available | Not available | Sanitized export |
| Raw historic import | Not available | Not available | Explicit `--full` export |
| Context injection | Prompt argument | Prompt file | Prompt argument |
| Resume bridge-linked sessions | Yes | Yes | Yes |

Run `cb doctor --verbose` before importing or continuing. Unsupported or
unknown profiles degrade safely to a manual handoff instead of modifying source
sessions.

## Privacy by default

Context Bridge is built for work that should not be silently copied into
another vendor's context window.

- Local SQLite storage with WAL, foreign keys, embedded migrations, and
  restrictive file permissions
- Strict secret classification before a target prompt is generated
- Known credential paths excluded from file context
- Repository configuration treated as untrusted unless you explicitly opt in

Use `cb timeline <bridge-session-id>` for local inspection. If a handoff must
leave your machine, export a redacted copy first:

```bash
cb export <bridge-session-id> --format json --redacted > session-context.json
```

An unredacted export is an explicit local operation and may contain sensitive
canonical events. Read the [privacy and security guide](docs/privacy-and-security.md)
before sharing one.

## Command map

```text
cb run <codex|claude|opencode>              Launch and record observable work
cb import opencode [--session <id>] [--full] Bring in a supported session
cb continue --session <id> --to <agent>     Generate and launch a handoff
cb continue --last --from <agent> --to <agent>
cb sessions | show <id> | timeline <id>     Inspect local bridge data
cb diff <id> | checkpoint [--note <text>]   Review changes or mark progress
cb export <id> --format <markdown|json>     Export local canonical context
cb integrate claude [--remove]              Manage the opt-in Claude hook
cb doctor --verbose                         Check agents and profiles
```

Useful global flags:

```text
--project <path>       Operate on another repository
--data-dir <path>      Override the local Context Bridge data directory
--config <path>        Use an explicit trusted configuration file
--trust-project-config Allow privileged repository-local configuration
--json                 Emit machine-readable output
```

## Configuration

Configuration resolves in this order:

1. Built-in defaults
2. Global config
3. `<project>/.context-bridge.toml`
4. An explicit `--config` file
5. `CB_*` environment overrides
6. CLI arguments

```toml
[general]
default_target = "claude"
context_budget = 40000
preview_before_handoff = false

[security]
redaction = "strict"
excluded_paths = [".env", ".env.*", "**/secrets/**", "~/.ssh/**"]

[summarization]
mode = "deterministic"
```

Repository-local configuration is untrusted by default. Without
`--trust-project-config`, it may set only `general.default_target` and
`general.context_budget`; executable paths, storage, and redaction policy stay
protected. Inspect your active configuration with:

```bash
cb config show
cb config path
```

## Install from source

Rust `1.89` or newer is supported; `rust-toolchain.toml` pins the development
toolchain used by CI.

```bash
cargo build --release -p cb-cli
cargo install --path crates/cb-cli --locked
cb doctor --verbose
```

Data is stored in the platform-appropriate application data directory, or in
`CB_DATA_DIR` when that environment variable is set.

## Contributing

Context Bridge is Rust-first and intentionally conservative around external
agent formats. New adapters need documented protocol evidence, guarded version
profiles, fixtures, and failure modes that preserve the source session.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo deny check
```

The integration suite creates temporary repositories and fake agent
executables to exercise full continuation and crash-safety paths without real
agent credentials.

- [Contributing](CONTRIBUTING.md)
- [Architecture](docs/architecture.md)
- [Canonical context format](docs/canonical-context-format.md)
- [Adapter development](docs/adapter-development.md)
- [Compatibility](docs/compatibility.md)
- [Session import guide](docs/import.md)
- [Privacy and security](docs/privacy-and-security.md)
- [Security policy](SECURITY.md)

## License

MIT © Context Bridge contributors. See [LICENSE](LICENSE).
