# Adapter development

Adapters implement `cb_core::AgentAdapter` and must report capabilities from a
detected installation and compatibility profile. The agent name alone is never
evidence that a parser, resume flag, or server interface is safe.

## Integration preference

1. Official API, SDK, hook, server interface, or export command
2. Stable documented local session storage
3. Versioned parser for a known local format
4. Optional PTY observation for coarse lifecycle signals only

ANSI terminal output is never a canonical transcript. Adapters must not modify
vendor-owned session files.

## Required profile data

Each enabled profile records:

- Agent version requirement
- Capability matrix
- Parser/schema version
- Discovery and source paths
- Fixture versions covered
- Native launch/resume syntax from vendor documentation
- Safe fallback when the profile is not applicable

Unknown versions must disable unsafe parsing. Detection may still report the
installation and permit an ordinary interactive launch when no context
injection is requested.

## Mapping pipeline

A vendor parser emits versioned raw records. A mapper converts them to typed
`RawEvent` values. The shared normalizer attaches provenance, hashes,
sensitivity, and stable import keys before events cross into the domain.

Do not place `sqlx` rows, process handles, or vendor JSON structures in
`cb-core`.

## Import requirements

- Treat external storage as read-only.
- Bound reads and large outputs.
- Preserve unknown optional vendor fields in canonical `metadata`.
- Skip isolated malformed records with a diagnostic.
- Fail safely when the entire source is unsupported or malformed.
- Namespace import keys by external session.
- Exercise duplicate refreshes and database rollback.

## Fixture checklist

For every supported version, include anonymized:

- A normal session
- Tool call/result pairs
- File and command events
- Unknown optional fields
- Large-output references
- Secret-containing content
- Malformed/truncated records

Golden tests cover normalized events and target handoffs. CI must not require an
installed real CLI or credentials. Real compatibility tests belong in ignored
developer tests or scripts and must never alter the source session.

## Hook installation

Hook integration is opt-in through `cb integrate <agent>`. Before any write:

1. Detect and validate the installed version.
2. Parse existing configuration without dropping unknown fields.
3. Back up the exact file.
4. Add only the Context Bridge hook.
5. Make repeated installation idempotent.
6. Implement `--remove` without touching unrelated hooks.

The Claude Code 2.1 profile implements this contract for a project-local
`SessionEnd` command hook. It performs the version/capability check before any
write, retains a timestamped backup, and never adds a global hook implicitly.
