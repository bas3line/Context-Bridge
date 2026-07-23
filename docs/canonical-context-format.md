# Canonical context format

The canonical format is vendor-neutral and schema-versioned. Rust types in
`cb-core` are authoritative; JSON exports use their Serde representation.

## Identifiers

Bridge sessions, events, checkpoints, and handoffs use UUIDv7 identifiers.
External session IDs remain opaque validated strings. Projects use a BLAKE3
hash of the canonical Git root, or the normalized absolute path when Git is
unavailable.

## Event envelope

Every stored event contains:

```json
{
  "id": "UUIDv7",
  "bridge_session_id": "UUIDv7",
  "source_agent": "claude",
  "external_event_id": "opaque vendor event id",
  "sequence": 42,
  "timestamp": "2026-07-24T00:00:00Z",
  "kind": "test_run",
  "payload": {
    "type": "test_run",
    "data": {
      "command": "cargo test",
      "outcome": "passed",
      "summary": "All tests passed."
    }
  },
  "content_hash": "BLAKE3 hex",
  "sensitivity": "normal",
  "import_metadata": {
    "parser_name": "opencode-export",
    "parser_version": "opencode-export-v1",
    "source_path": "/local/read-only/source",
    "imported_at": "2026-07-24T00:00:01Z"
  },
  "parent_event_id": null,
  "metadata": {}
}
```

`metadata` is reserved for versioned vendor fields that cannot yet be
normalized. Important handoff facts use typed payload variants rather than
arbitrary JSON.

## Event kinds

Messages, tool calls/results, commands, file reads/creates/modifications/
deletions/moves, Git state/diffs, tests, decisions, assumptions, errors,
checkpoints, and handoffs are represented directly.

File payloads store normalized paths and optional move origins. Large tool
results store a summary, hash, original event reference, and optional local
artifact path rather than copying the full output.

Each `git_state` payload also carries the captured filesystem file count and a
BLAKE3 fingerprint over its policy-filtered metadata. This records the initial
and reconciled filesystem state without duplicating file contents; individual
created, modified, deleted, and moved file events describe the delta.

## Ordering and deduplication

`sequence` is unique within a bridge session and allocated in the same
transaction as insertion. `import_key` is unique per session. A partial unique
index also prevents duplicate `(session, source agent, external event ID)`
tuples.

Content hashes establish payload identity and artifact provenance; they are not
used alone for deduplication because identical user messages may be legitimate
separate events.

## Sensitivity

- `normal`: safe under the configured policy
- `potential_secret`: included only after deterministic redaction
- `secret`: stored locally for provenance but excluded from handoffs
- `excluded`: path or content policy forbids use

No secret or excluded event is passed to a target renderer.

## Handoff schema

`HandoffPackage.schema_version` currently equals `1`. It contains source/target
agents, project and Git state, original/current objectives, completed/current
work, decisions, assumptions, failed approaches, file context, commands, tests,
errors, pending tasks, recommended action, and recent conversation.

Budget compaction removes lower-priority material first. If the requested
budget cannot contain the required objective and constraints, generation fails
with the minimum approximate requirement instead of silently exceeding the
limit.
