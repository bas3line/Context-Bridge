# Context Bridge

> Local-first, redacted context handoffs between AI coding agents.

Context Bridge (`cb`) is a local-first context synchronization CLI for AI coding
agents. It records observable session events in a vendor-neutral, append-only
log and derives redacted handoff packages so work can continue in another agent
without pretending to transfer hidden model state, private reasoning, or KV
caches.

**Status:** early `0.1.0` release. The canonical event model and privacy
boundary are stable enough to evaluate locally; adapter compatibility remains
intentionally narrow and version-gated.

## Why Context Bridge?

Coding sessions are often stranded in one CLI even when the next task needs a
different model or agent. **Context Bridge preserves the observable work, not
the illusion of a perfect session clone.** It gives the next agent a reviewed
brief containing the task, repository state, useful conversation, and recorded
evidence while keeping the canonical event log on the operator's machine.

- **Local-first:** no telemetry, upload service, or vendor session mutation.
- **Redaction-aware:** secret-classified events and excluded paths stay out of
  generated target prompts.
- **Evidence-based:** commands, tests, Git state, checkpoints, and provenance
  are explicit instead of inferred from a terminal scrape.
- **Version-gated:** documented adapter profiles are supported; unknown profiles
  fall back safely instead of reading private agent databases.

## Quick start

Install from a local checkout with the pinned Rust toolchain, then verify the
agent profiles on the machine before moving any session:

```bash
cargo install --path crates/cb-cli --locked
cb doctor --verbose
```

Start a bridge-managed OpenCode session, or import an existing supported session:

```bash
cb run opencode

cb import opencode --session <opencode-session-id>
cb continue --session <bridge-session-id> --from opencode --to claude --preview
cb continue --session <bridge-session-id> --from opencode --to claude
```

Always run `--preview` before launching a handoff. See
[the import guide](docs/import.md) for raw-export boundaries, recency-first
context budgets, and target-agent continuation details.

The current `0.1.0` milestone provides the canonical model, SQLite storage,
project and Git reconciliation, deterministic context reduction, redaction,
inspection/export commands, and fixture-driven end-to-end handoffs. Production
adapters are enabled only for explicitly versioned, documented CLI profiles:
OpenCode 1.18 has structured session discovery and sanitized JSON export;
Claude Code 2.1 and Codex CLI 0.145 support documented bootstrap prompts and
native resume. Unknown versions degrade to a manual handoff file rather than
guessing undocumented flags or private storage formats.

## What is transferred

- User, assistant, and system messages
- Structured tool calls and relevant results
- Commands, tests, decisions, assumptions, and errors
- Files inspected or changed
- Git branch, HEAD, status, staged/unstaged diffs, and untracked paths
- Checkpoints, completed work, pending tasks, and the recommended next action
- Provenance, content hashes, parser versions, and source event identifiers

Generated summaries are derived data. The append-only canonical event log is
the source of truth and can reproduce a handoff.

## Install from source

Rust 1.89 or newer is supported; `rust-toolchain.toml` pins the development
toolchain used by CI.

```bash
cargo build --release -p cb-cli
cargo install --path crates/cb-cli --locked
cb doctor --verbose
```

No telemetry or network service is enabled. Data is stored in the
platform-appropriate application data directory, or at `CB_DATA_DIR` when set.

## CLI

```text
cb [--trust-project-config] <command>
cb run <codex|claude|opencode>
cb continue [--session <id> | --last] [--from <agent>] --to <agent>
cb import <agent> [--session <external-id>]
cb sessions
cb show <session-id>
cb timeline <session-id> [--include-sensitive]
cb diff <session-id>
cb checkpoint [--note <text>]
cb export <session-id> --format <markdown|json> [--redacted]
cb integrate <agent> [--remove]
cb doctor --verbose
cb config show
cb config path
cb config set security.redaction strict
```

Use `cb continue --to opencode --preview` to render a handoff without launching
the target. `--budget` overrides the approximate deterministic token budget.

`cb timeline` omits non-normal events by default; pass `--include-sensitive`
only for private local inspection.

`cb run` inherits the caller's controlling terminal directly. It does not parse
ANSI output or insert a fake pipe between the user and the agent. After the
child exits, it always reconciles the filesystem and Git state. Structured
records are imported only by an adapter with an explicitly supported,
documented protocol. Currently that includes OpenCode 1.18's sanitized JSON
export; the fixture-only Fake JSONL v1 profile remains in the test suite.
Claude and Codex do not expose a documented structured transcript-import path
in their guarded profiles, so Context Bridge captures their observable project
state and resumes only bridge-linked sessions through documented CLI commands.

## Configuration

Configuration is layered in this order:

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

[storage]
data_dir = "~/.local/share/context-bridge"

[security]
redaction = "strict"
excluded_paths = [".env", ".env.*", "**/secrets/**", "~/.ssh/**"]

[agents.claude]
executable = "claude"

[agents.codex]
executable = "codex"

[agents.opencode]
executable = "opencode"

[summarization]
mode = "deterministic"
```

Unknown fields are rejected. Environment-variable values are not captured or
stored.

Repository-local `.context-bridge.toml` is untrusted by default. Without
`--trust-project-config`, it may only set `general.default_target` and
`general.context_budget`; storage locations, redaction policy, agent
executables, and other privileged settings are rejected. Pass the flag only
after reviewing the repository configuration (or explicitly select a config
file you trust with `--config`).

## Compatibility status

| Capability | Fake JSONL v1 | Codex CLI | Claude Code | OpenCode |
| --- | --- | --- | --- | --- |
| Detection/version | Yes | Codex CLI 0.145 | Claude Code 2.1 | OpenCode 1.18 |
| Interactive launch | Yes | Yes | Yes | Yes |
| Structured import | Yes | Disabled | Disabled | Sanitized export |
| Context injection | Yes | Prompt argument | Prompt argument | Prompt argument |
| Native resume | Yes | Yes | Yes | Yes |
| Hook/config changes | Not applicable | None | Optional project hook | None |

The fake protocol is test-only and enabled by `CB_TEST_MODE=true`. A fake
executable receives paths through `CB_EVENT_SINK`, `CB_SESSION_METADATA`, and
`CB_BOOTSTRAP_PATH`; production integrations never receive these variables.

`cb integrate claude` installs one project-local `SessionEnd` command hook in
`.claude/settings.local.json`, backs up pre-existing settings, and preserves
unrelated hook groups. `cb integrate claude --remove` removes only that hook.

## Security defaults

- Local-only storage with restrictive directory and SQLite permissions
- WAL, foreign keys, embedded migrations, and crash-safe transactions
- Strict secret classification before handoff
- Known credential paths excluded from file context
- Secret-classified events never enter a target prompt
- No raw terminal logging, telemetry, or uploads
- No modification of vendor-owned session records

An unredacted export is an explicit local operation and may contain sensitive
canonical events. Prefer `--redacted` before sharing an export. See
[privacy and security](docs/privacy-and-security.md) for the threat model.

## Development

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo deny check
```

The integration test creates a temporary Git repository and fake executables,
then proves the complete continuation and crash-safety scenario without real
agent credentials.

Architecture and extension details live in [docs/architecture.md](docs/architecture.md)
and [docs/adapter-development.md](docs/adapter-development.md). For the
practical OpenCode-to-Claude/Codex workflow, see
[docs/import.md](docs/import.md).

## Documentation and community

- [Architecture](docs/architecture.md) explains the canonical event model,
  storage boundary, and deterministic handoff reduction.
- [Compatibility](docs/compatibility.md) lists the guarded agent profiles and
  their supported capabilities.
- [Privacy and security](docs/privacy-and-security.md) documents redaction,
  excluded paths, and safe local exports.
- [Contributing](CONTRIBUTING.md) covers development gates and parser-fixture
  expectations.
- [Security policy](SECURITY.md) explains how to report a vulnerability without
  disclosing session data or credentials.

Context Bridge is released under the [MIT License](LICENSE).
