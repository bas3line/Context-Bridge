# Privacy and security

Context Bridge handles source code, terminal activity, and private
conversations. Its default trust boundary is one local user account.

## Default guarantees

- No telemetry, uploads, remote server, or model API
- Platform data directory with mode `0700` on Unix
- SQLite database, WAL, and shared-memory files restricted to mode `0600`
- No environment-variable value capture
- No raw PTY stream logging
- No modification of external-agent session files
- Secret scanning before handoff
- Deterministic redaction and path exclusion

Default excluded paths cover `.env` files, SSH material, cloud credentials,
private keys, secret directories, keychains, and browser data. Users may add
project-specific globs.

Context Bridge applies restrictive modes only to application directories it
creates. It refuses an existing data or configuration directory that is
accessible by group or other users rather than changing the permissions of an
arbitrary parent such as `/tmp`.

## Threat model

The current milestone protects against accidental cross-agent credential
disclosure and accidental sharing of known secret files. It does not protect
against an attacker who already controls the local account, the target agent,
or the project repository.

Strict scanning is heuristic. False positives are preferable in target prompts;
the canonical local event preserves provenance and its sensitivity label.
Potential secrets are redacted line by line. Secret and excluded events never
enter a handoff package.

## Exports

`cb export` is local. The default unredacted JSON form can contain sensitive
events and is intended for private backup/debugging. Use `--redacted` before
sharing. Markdown exports omit excluded paths; redacted Markdown also replaces
secret events.

## Git and files

Context Bridge runs only read-only Git inspection commands. It never resets,
checks out, cleans, stages, commits, or pushes. File tracking records metadata
and hashes small files; it does not duplicate file contents when a Git diff is
sufficient. Diffs are bounded before storage and compacted again for handoffs.

## Optional encryption

At-rest encryption is reserved for a feature using an established library and
OS-backed key management. The project intentionally contains no custom
cryptography.

## Reporting

Do not open a public issue containing a real transcript, database, credential,
or private repository path. Follow [SECURITY.md](../SECURITY.md) for private
reporting.
