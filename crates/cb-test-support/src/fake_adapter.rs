use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct FakeAgentExecutable {
    pub path: PathBuf,
}

impl FakeAgentExecutable {
    pub fn install(directory: &Path, name: &str) -> std::io::Result<Self> {
        fs::create_dir_all(directory)?;
        let path = directory.join(name);
        let script = r#"#!/bin/sh
trap 'exit 130' INT
trap 'exit 143' TERM
if [ "${1:-}" = "--version" ]; then
  echo "context-bridge fake agent 1.0.0"
  exit 0
fi
if [ -n "${CB_FAKE_SOURCE_EVENTS:-}" ]; then
  cp "$CB_FAKE_SOURCE_EVENTS" "$CB_EVENT_SINK"
fi
if [ -n "${CB_FAKE_APPEND_EVENTS:-}" ]; then
  cat "$CB_FAKE_APPEND_EVENTS" >> "$CB_EVENT_SINK"
fi
if [ -n "${CB_FAKE_MODIFY_FILE:-}" ]; then
  printf '%s\n' "${CB_FAKE_MODIFY_CONTENT:-modified by fake agent}" > "$CB_FAKE_MODIFY_FILE"
fi
if [ -n "${CB_BOOTSTRAP_PATH:-}" ] && [ -n "${CB_FAKE_CAPTURE_BOOTSTRAP:-}" ]; then
  cp "$CB_BOOTSTRAP_PATH" "$CB_FAKE_CAPTURE_BOOTSTRAP"
fi
session_id="${CB_FAKE_EXTERNAL_SESSION_ID:-fake-session}"
if [ -n "${CB_FAKE_MALFORMED_SESSION_METADATA:-}" ]; then
  printf '%s\n' "$CB_FAKE_MALFORMED_SESSION_METADATA" > "$CB_SESSION_METADATA"
else
  printf '{"external_session_id":"%s"}\n' "$session_id" > "$CB_SESSION_METADATA"
fi
if [ -n "${CB_FAKE_READY_PATH:-}" ]; then
  : > "$CB_FAKE_READY_PATH"
fi
if [ -n "${CB_FAKE_SLEEP_SECONDS:-}" ]; then
  sleep "$CB_FAKE_SLEEP_SECONDS"
fi
exit "${CB_FAKE_EXIT_CODE:-0}"
"#;
        fs::write(&path, script)?;
        set_executable(&path)?;
        Ok(Self { path })
    }
}

#[cfg(unix)]
fn set_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> std::io::Result<()> {
    Ok(())
}
