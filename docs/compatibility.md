# Compatibility

Context Bridge targets macOS and Linux first. Windows abstractions remain
portable but are not yet validated.

## Guarded production profiles

| Profile | Detection | Launch | Discovery/import | Handoff injection | Resume |
| --- | --- | --- | --- | --- | --- |
| `fake-jsonl-v1` | Yes | Yes | Yes | Yes | Yes |
| `opencode-cli-1.18` | Yes | New and resumed sessions | Sanitized JSON export | Yes | Yes |
| `claude-cli-2.1` | Yes | New and resumed sessions | No documented export parser | Yes | Yes |
| `codex-cli-0.145` | Yes | New and resumed sessions | No documented export parser | Yes | Yes |
| `unverified-launch-only` | No | No bootstrap | No | No | No |

The fake profile is enabled only by `CB_TEST_MODE=true`. It exists to prove the
architecture and acceptance scenario without credentials.

OpenCode 1.18 uses only `session list --format json`, `export <id> --sanitize`,
and documented prompt/session flags. Claude Code 2.1 uses `--session-id` for a
bridge-created session and `--resume` for an existing linked session. Codex CLI
0.145 uses its documented prompt argument and `resume <id>` command. No profile
reads private vendor storage or parses terminal output. Unknown versions reject
bootstrap/resume and produce a redacted manual handoff under the Context Bridge
data directory instead.

Claude's optional `cb integrate claude` installs a project-local `SessionEnd`
hook only after explicit user action. It is backup-safe and removal is scoped to
Context Bridge's own command handler.

## Diagnostic contract

`cb doctor --verbose` reports the executable, version, compatibility profile,
capabilities, database/schema health, project root, Git availability, config
warnings, and permission warnings. It never prints configuration secrets or
environment values.
