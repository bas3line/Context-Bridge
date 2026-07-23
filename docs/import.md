# Move a session to another agent

Context Bridge stores a local canonical copy of supported session data, then
builds a redacted handoff for the next agent. It transfers observable messages,
tool activity, and current project state; it does not transfer hidden model
state, private reasoning, or provider-side caches.

## OpenCode to Claude Code

OpenCode 1.18 is the supported historic-session import source. First check that
the installed tools have a guarded compatibility profile:

```bash
cb doctor --verbose
```

Import an OpenCode session by its external ID. This reads only the documented,
sanitized `opencode export <id> --sanitize` output and does not modify the
source session.

```bash
cb import opencode --session <opencode-session-id>
```

The safe default uses OpenCode's sanitized export. If you need the original
local text (including assistant reasoning/output that OpenCode sanitizes), opt
in explicitly. Review the resulting bridge data before sharing it: raw exports
may contain credentials or other sensitive text.

```bash
cb import opencode --session <opencode-session-id> --full
```

Copy the printed `bridge_session_id`. Preview the handoff before launching
Claude:

```bash
cb continue --session <bridge-session-id> --to claude --preview
```

Then open Claude with the handoff:

```bash
cb continue --session <bridge-session-id> --to claude
```

For example:

```bash
cb import opencode --session ses_abc123
# Bridge session: 019f0000-0000-7000-8000-000000000000

cb continue --session 019f0000-0000-7000-8000-000000000000 --to claude
```

## Use the OpenCode session deck

In a terminal, `cb run opencode` opens a small menu instead of launching or
scanning immediately. Choose `2` / `import` to list and select an existing
OpenCode session, or choose `1` / `new` when starting fresh work.

```bash
cb run opencode
```

## Continue a bridge-managed session

When the source session was started or imported through Context Bridge, use
`continue` to move it to any detected target profile:

```bash
cb continue --last --from opencode --to codex
cb continue --session <bridge-session-id> --to claude
```

`--preview` renders the handoff without starting the target agent.

## Context size and full history

Import preserves the complete supported source export in the local canonical
event log. `--full` preserves the documented raw export; without it OpenCode's
own sanitizer may replace text with placeholders. The handoff prompt still has
a finite context budget because the target model has a finite context window.
`--budget` changes the maximum size of that generated prompt; it does not
delete imported history. Context Bridge fills that budget recency-first: it
keeps the newest supported conversation that fits, plus the current objective
and structured repository state. It does not start at the beginning of a long
session.

For Claude Code, the handoff is passed through its documented
`--append-system-prompt-file` interface, not as one shell argument. This makes
large handoffs such as 100,000 tokens practical without hitting the operating
system's command-line length limit. The temporary file is private and is
removed after Claude exits.

```bash
cb continue --session <bridge-session-id> --to claude --budget 100000 --preview
```

Use `cb timeline <bridge-session-id>` to inspect canonical events locally, or
export them for local review:

```bash
cb export <bridge-session-id> --format json --redacted > session-context.json
```

## Current support boundary

Claude Code and Codex support documented new-session prompts and native resume
for sessions already linked by Context Bridge. They do not currently expose a
documented structured historic-session export, so `cb import claude` and
`cb import codex` are intentionally unavailable. Start those agents through
Context Bridge when you need future resume/handoff support.
