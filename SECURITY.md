# Security policy

## Supported versions

Security fixes are applied to the latest released minor version. The current
pre-1.0 API may change while preserving migration safety.

## Reporting a vulnerability

Use the repository's private security-advisory flow. Do not file a public issue
with a session export, SQLite database, credential, private source file, or
identifying project path.

Include:

- A minimal redacted reproduction
- Context Bridge version and OS
- Affected adapter compatibility profile
- Whether an external agent session was modified
- Whether a secret entered a handoff/export or file permissions were weakened

You should receive an acknowledgement within five business days. No telemetry
or automatic upload is available, so maintainers may request a deliberately
sanitized fixture.

## Scope

High-priority reports include secret-classified content entering a target
prompt, path-exclusion bypasses, unsafe vendor-file modification, SQLite
permission regressions, parser crashes on untrusted records, or command
injection through adapter arguments.
