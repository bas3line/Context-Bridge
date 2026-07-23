use cb_core::AgentKind;
use miette::IntoDiagnostic;
use serde::Serialize;

use crate::{
    commands::{App, config::SummarizationMode},
    output::{print_json, print_table, terminal_safe, terminal_single_line},
};

#[derive(Debug, Serialize)]
struct DoctorReport {
    context_bridge_version: &'static str,
    os: &'static str,
    architecture: &'static str,
    data_directory: String,
    database_health: String,
    schema_version: String,
    project_root: String,
    git: String,
    agents: Vec<AgentReport>,
    configuration_warnings: Vec<String>,
    permission_warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct AgentReport {
    agent: AgentKind,
    executable: Option<String>,
    version: Option<String>,
    compatibility_profile: Option<String>,
    capabilities: Option<cb_core::AgentCapabilities>,
    session_storage: String,
    integration_status: String,
    error: Option<String>,
}

pub async fn execute(app: &App, verbose: bool) -> miette::Result<i32> {
    let database_health = match app.store.health_check().await {
        Ok(()) => "healthy".to_owned(),
        Err(error) => format!("unhealthy: {error}"),
    };
    let schema_version = app.store.schema_version().await.into_diagnostic()?;
    let git = tokio::process::Command::new("git")
        .arg("--version")
        .output()
        .await
        .ok()
        .filter(|output| output.status.success())
        .map_or_else(
            || "not available".to_owned(),
            |output| String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        );
    let mut agents = Vec::new();
    for agent in [AgentKind::Codex, AgentKind::ClaudeCode, AgentKind::OpenCode] {
        let adapter = app.adapters.get(agent).into_diagnostic()?;
        match adapter.detect().await {
            Ok(installation) => {
                let capabilities = adapter.capabilities().await.into_diagnostic()?;
                agents.push(AgentReport {
                    agent,
                    executable: Some(installation.executable.display().to_string()),
                    version: installation.version,
                    compatibility_profile: Some(installation.compatibility_profile),
                    capabilities: verbose.then_some(capabilities.clone()),
                    session_storage: session_storage_status(app, &capabilities),
                    integration_status: integration_status(app, agent),
                    error: None,
                });
            }
            Err(error) => agents.push(AgentReport {
                agent,
                executable: None,
                version: None,
                compatibility_profile: None,
                capabilities: None,
                session_storage: "unavailable because executable detection failed".to_owned(),
                integration_status: "not installed".to_owned(),
                error: Some(error.to_string()),
            }),
        }
    }
    let mut configuration_warnings = Vec::new();
    if matches!(app.config.summarization.mode, SummarizationMode::External) {
        configuration_warnings.push(
            "external summarization is configured but not implemented; deterministic mode remains active"
                .to_owned(),
        );
    }
    if matches!(
        app.config.security.redaction,
        super::config::RedactionMode::Off
    ) {
        configuration_warnings
            .push("secret redaction is disabled; handoff safety is reduced".to_owned());
    }
    let permission_warnings = permission_warnings(&app.data_dir);
    let report = DoctorReport {
        context_bridge_version: env!("CARGO_PKG_VERSION"),
        os: std::env::consts::OS,
        architecture: std::env::consts::ARCH,
        data_directory: app.data_dir.display().to_string(),
        database_health,
        schema_version,
        project_root: app.project.root.display().to_string(),
        git,
        agents,
        configuration_warnings,
        permission_warnings,
    };
    if app.json {
        print_json(&report)?;
    } else {
        println!(
            "Context Bridge {}",
            terminal_safe(report.context_bridge_version)
        );
        println!(
            "OS: {} {}",
            terminal_safe(report.os),
            terminal_safe(report.architecture)
        );
        println!(
            "Data directory: {}",
            terminal_single_line(&report.data_directory)
        );
        println!(
            "Database: {}",
            terminal_single_line(&report.database_health)
        );
        println!("Schema: {}", terminal_single_line(&report.schema_version));
        println!(
            "Project root: {}",
            terminal_single_line(&report.project_root)
        );
        println!("Git: {}", terminal_single_line(&report.git));
        let rows = report
            .agents
            .iter()
            .map(|agent| {
                vec![
                    agent.agent.to_string(),
                    agent.executable.clone().unwrap_or_else(|| "-".to_owned()),
                    agent.version.clone().unwrap_or_else(|| "-".to_owned()),
                    agent
                        .compatibility_profile
                        .clone()
                        .unwrap_or_else(|| "-".to_owned()),
                    agent.session_storage.clone(),
                    agent.integration_status.clone(),
                    agent.error.clone().unwrap_or_else(|| "ok".to_owned()),
                ]
            })
            .collect::<Vec<_>>();
        print_table(
            &[
                "AGENT",
                "EXECUTABLE",
                "VERSION",
                "PROFILE",
                "SESSION STORAGE",
                "INTEGRATION",
                "STATUS",
            ],
            &rows,
        );
        for warning in report
            .configuration_warnings
            .iter()
            .chain(&report.permission_warnings)
        {
            eprintln!("warning: {}", terminal_safe(warning));
        }
    }
    Ok(i32::from(report.database_health != "healthy"))
}

fn session_storage_status(app: &App, capabilities: &cb_core::AgentCapabilities) -> String {
    if app.config.test_mode {
        "test fixture discovery enabled".to_owned()
    } else if capabilities.structured_export {
        "documented structured export enabled".to_owned()
    } else if capabilities.session_discovery {
        "documented session discovery enabled; import is unavailable".to_owned()
    } else {
        "no documented structured session storage interface enabled".to_owned()
    }
}

fn integration_status(app: &App, agent: AgentKind) -> String {
    if agent != AgentKind::ClaudeCode {
        return "no vendor configuration changes required".to_owned();
    }
    let path = app.project.root.join(".claude/settings.local.json");
    match std::fs::read_to_string(&path) {
        Ok(contents) if contents.contains("Claude Code SessionEnd observed by Context Bridge") => {
            format!("project SessionEnd hook installed at `{}`", path.display())
        }
        Ok(_) => "optional project SessionEnd hook is not installed".to_owned(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            "optional project SessionEnd hook is not installed".to_owned()
        }
        Err(_) => "could not inspect optional project hook settings".to_owned(),
    }
}

#[cfg(unix)]
fn permission_warnings(data_dir: &std::path::Path) -> Vec<String> {
    use std::os::unix::fs::PermissionsExt;
    let mut warnings = Vec::new();
    if let Ok(metadata) = data_dir.metadata()
        && metadata.permissions().mode() & 0o077 != 0
    {
        warnings.push(format!(
            "data directory `{}` is accessible by group or other users",
            data_dir.display()
        ));
    }
    warnings
}

#[cfg(not(unix))]
fn permission_warnings(_data_dir: &std::path::Path) -> Vec<String> {
    Vec::new()
}
