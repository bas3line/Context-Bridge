use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use cb_core::AgentKind;
use chrono::Utc;
use miette::{IntoDiagnostic, WrapErr, miette};
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::{
    commands::App,
    output::{print_field, print_json},
};

const CLAUDE_SESSION_END_COMMAND: &str = "cb checkpoint --note 'Claude Code SessionEnd observed by Context Bridge' >/dev/null 2>&1 || true";

#[derive(Debug, Serialize)]
struct IntegrationResult {
    agent: AgentKind,
    action: &'static str,
    settings_path: PathBuf,
    backup_path: Option<PathBuf>,
    changed: bool,
}

pub async fn execute(app: &App, agent: AgentKind, remove: bool) -> miette::Result<i32> {
    if agent != AgentKind::ClaudeCode {
        return Err(miette!(
            "{agent} has no configuration-writing integration. Its documented CLI profile works without modifying vendor files."
        ));
    }
    let adapter = app.adapters.get(agent).into_diagnostic()?;
    let installation = adapter.detect().await.into_diagnostic()?;
    let capabilities = adapter.capabilities().await.into_diagnostic()?;
    if !capabilities.lifecycle_hooks {
        return Err(miette!(
            "Claude profile `{}` is not verified for managed SessionEnd hooks; run `cb doctor --verbose` and upgrade Claude Code before modifying settings",
            installation.compatibility_profile
        ));
    }
    let settings_path = app.project.root.join(".claude/settings.local.json");
    let result = manage_claude_session_end_hook(&settings_path, remove)?;
    if app.json {
        print_json(&result)?;
    } else {
        print_field("Agent", result.agent);
        print_field("Action", result.action);
        print_field("Settings", result.settings_path.display());
        print_field("Changed", result.changed);
        if let Some(backup) = result.backup_path {
            print_field("Backup", backup.display());
        }
    }
    Ok(0)
}

fn manage_claude_session_end_hook(path: &Path, remove: bool) -> miette::Result<IntegrationResult> {
    let existed = path.exists();
    if existed {
        let metadata = fs::symlink_metadata(path).into_diagnostic()?;
        if !metadata.file_type().is_file() {
            return Err(miette!(
                "refusing to modify `{}` because it is not a regular settings file",
                path.display()
            ));
        }
    }
    let original = if existed {
        fs::read(path)
            .into_diagnostic()
            .wrap_err_with(|| format!("could not read `{}`", path.display()))?
    } else {
        Vec::new()
    };
    let mut document = if existed {
        serde_json::from_slice::<Value>(&original)
            .into_diagnostic()
            .wrap_err_with(|| {
                format!(
                    "refusing to overwrite invalid Claude settings `{}`; repair the JSON first",
                    path.display()
                )
            })?
    } else {
        json!({})
    };
    let root = document.as_object_mut().ok_or_else(|| {
        miette!(
            "refusing to modify `{}` because its root is not a JSON object",
            path.display()
        )
    })?;
    let hooks = object_field(root, "hooks", path)?;
    let session_end = array_field(hooks, "SessionEnd", path)?;
    let changed = if remove {
        remove_our_hook(session_end)
    } else {
        install_our_hook(session_end)
    };
    if !changed {
        return Ok(IntegrationResult {
            agent: AgentKind::ClaudeCode,
            action: if remove { "remove" } else { "install" },
            settings_path: path.to_path_buf(),
            backup_path: None,
            changed: false,
        });
    }
    let backup_path = existed.then(|| backup_path(path));
    if let Some(backup_path) = &backup_path {
        fs::write(backup_path, &original)
            .into_diagnostic()
            .wrap_err_with(|| {
                format!(
                    "could not write settings backup `{}`",
                    backup_path.display()
                )
            })?;
    }
    atomic_write(
        path,
        &serde_json::to_vec_pretty(&document).into_diagnostic()?,
    )?;
    Ok(IntegrationResult {
        agent: AgentKind::ClaudeCode,
        action: if remove { "remove" } else { "install" },
        settings_path: path.to_path_buf(),
        backup_path,
        changed: true,
    })
}

fn object_field<'a>(
    object: &'a mut Map<String, Value>,
    key: &str,
    path: &Path,
) -> miette::Result<&'a mut Map<String, Value>> {
    let value = object.entry(key.to_owned()).or_insert_with(|| json!({}));
    value.as_object_mut().ok_or_else(|| {
        miette!(
            "refusing to modify `{}` because `{key}` is not a JSON object",
            path.display()
        )
    })
}

fn array_field<'a>(
    object: &'a mut Map<String, Value>,
    key: &str,
    path: &Path,
) -> miette::Result<&'a mut Vec<Value>> {
    let value = object.entry(key.to_owned()).or_insert_with(|| json!([]));
    value.as_array_mut().ok_or_else(|| {
        miette!(
            "refusing to modify `{}` because `{key}` is not a JSON array",
            path.display()
        )
    })
}

fn install_our_hook(groups: &mut Vec<Value>) -> bool {
    if groups.iter().any(group_has_our_hook) {
        return false;
    }
    groups.push(json!({
        "matcher": "",
        "hooks": [{
            "type": "command",
            "command": CLAUDE_SESSION_END_COMMAND,
            "timeout": 5
        }]
    }));
    true
}

fn remove_our_hook(groups: &mut Vec<Value>) -> bool {
    let mut changed = false;
    for group in groups.iter_mut() {
        if let Some(hooks) = group.get_mut("hooks").and_then(Value::as_array_mut) {
            let before = hooks.len();
            hooks.retain(|hook| {
                hook.get("command").and_then(Value::as_str) != Some(CLAUDE_SESSION_END_COMMAND)
            });
            changed |= hooks.len() != before;
        }
    }
    let before = groups.len();
    groups.retain(|group| {
        group
            .get("hooks")
            .and_then(Value::as_array)
            .is_none_or(|hooks| !hooks.is_empty())
    });
    changed || before != groups.len()
}

fn group_has_our_hook(group: &Value) -> bool {
    group
        .get("hooks")
        .and_then(Value::as_array)
        .is_some_and(|hooks| {
            hooks.iter().any(|hook| {
                hook.get("command").and_then(Value::as_str) == Some(CLAUDE_SESSION_END_COMMAND)
            })
        })
}

fn backup_path(path: &Path) -> PathBuf {
    path.with_file_name(format!(
        "{}.context-bridge-{}.bak",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("settings.local.json"),
        Utc::now().timestamp_millis()
    ))
}

fn atomic_write(path: &Path, contents: &[u8]) -> miette::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| miette!("settings path `{}` has no parent", path.display()))?;
    fs::create_dir_all(parent).into_diagnostic()?;
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    if let Err(error) = fs::remove_file(&temporary)
        && error.kind() != ErrorKind::NotFound
    {
        return Err(error).into_diagnostic();
    }
    fs::write(&temporary, contents).into_diagnostic()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600)).into_diagnostic()?;
    }
    fs::rename(&temporary, path)
        .into_diagnostic()
        .wrap_err_with(|| format!("could not replace Claude settings `{}`", path.display()))
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::{CLAUDE_SESSION_END_COMMAND, manage_claude_session_end_hook};

    #[test]
    fn integration_is_idempotent_and_removes_only_its_own_hook() {
        let directory = tempdir().expect("temporary directory");
        let path = directory.path().join(".claude/settings.local.json");
        std::fs::create_dir_all(path.parent().expect("parent")).expect("settings directory");
        std::fs::write(&path, r#"{"hooks":{"SessionEnd":[{"matcher":"other","hooks":[{"type":"command","command":"other-command"}]}]}}"#).expect("seed settings");
        assert!(
            manage_claude_session_end_hook(&path, false)
                .expect("install")
                .changed
        );
        assert!(
            !manage_claude_session_end_hook(&path, false)
                .expect("repeat install")
                .changed
        );
        let installed = std::fs::read_to_string(&path).expect("settings");
        assert!(installed.contains(CLAUDE_SESSION_END_COMMAND));
        assert!(installed.contains("other-command"));
        assert!(
            manage_claude_session_end_hook(&path, true)
                .expect("remove")
                .changed
        );
        let removed = std::fs::read_to_string(&path).expect("settings");
        assert!(!removed.contains(CLAUDE_SESSION_END_COMMAND));
        assert!(removed.contains("other-command"));
    }
}
