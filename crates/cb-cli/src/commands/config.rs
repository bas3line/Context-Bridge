use std::path::{Path, PathBuf};

use clap::Subcommand;
use directories::ProjectDirs;
use miette::{IntoDiagnostic, WrapErr, miette};
use serde::{Deserialize, Serialize};

use crate::{
    commands::App,
    output::{print_json, terminal_safe, terminal_single_line},
};

#[derive(Debug, Clone, Subcommand)]
pub enum ConfigAction {
    /// Print the effective layered configuration.
    Show,
    /// Print the writable global or explicitly selected config path.
    Path,
    /// Set a supported configuration key.
    Set { key: String, value: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct AppConfig {
    pub general: GeneralConfig,
    pub storage: StorageConfig,
    pub security: SecurityConfig,
    pub agents: AgentsConfig,
    pub summarization: SummarizationConfig,
    #[serde(skip)]
    pub test_mode: bool,
}

/// The small subset of repository-local configuration that is safe to honor
/// without treating the repository as trusted code.
///
/// In particular, a checked-out repository must not be able to redirect local
/// storage, weaken redaction, or select an executable that `cb doctor` will
/// invoke. Users can opt into the complete project configuration with the
/// global `--trust-project-config` flag.
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct UntrustedProjectConfig {
    #[serde(default)]
    general: UntrustedProjectGeneralConfig,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct UntrustedProjectGeneralConfig {
    default_target: Option<AgentName>,
    context_budget: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GeneralConfig {
    pub default_target: AgentName,
    pub context_budget: usize,
    pub preview_before_handoff: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StorageConfig {
    pub data_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SecurityConfig {
    pub redaction: RedactionMode,
    pub excluded_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentsConfig {
    pub claude: AgentConfig,
    pub codex: AgentConfig,
    pub opencode: AgentConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentConfig {
    pub executable: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SummarizationConfig {
    pub mode: SummarizationMode,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentName {
    Claude,
    Codex,
    Opencode,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RedactionMode {
    Off,
    Standard,
    Strict,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SummarizationMode {
    Deterministic,
    External,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            default_target: AgentName::Claude,
            context_budget: 40_000,
            preview_before_handoff: false,
        }
    }
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            data_dir: default_data_dir(),
        }
    }
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            redaction: RedactionMode::Strict,
            excluded_paths: Vec::new(),
        }
    }
}

impl Default for AgentsConfig {
    fn default() -> Self {
        Self {
            claude: AgentConfig {
                executable: "claude".into(),
            },
            codex: AgentConfig {
                executable: "codex".into(),
            },
            opencode: AgentConfig {
                executable: "opencode".into(),
            },
        }
    }
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            executable: PathBuf::new(),
        }
    }
}

impl Default for SummarizationConfig {
    fn default() -> Self {
        Self {
            mode: SummarizationMode::Deterministic,
        }
    }
}

impl AppConfig {
    pub fn expanded_data_dir(&self) -> miette::Result<PathBuf> {
        expand_home(&self.storage.data_dir)
    }
}

pub async fn load_config(
    project_root: &Path,
    explicit: Option<&Path>,
    trust_project_config: bool,
) -> miette::Result<(AppConfig, PathBuf)> {
    let global = default_config_path();
    let project = project_root.join(".context-bridge.toml");
    let mut merged = toml::Value::try_from(AppConfig::default())
        .into_diagnostic()
        .wrap_err("could not construct built-in configuration")?;
    merge_file(&mut merged, &global, false).await?;
    if !explicit.is_some_and(|path| paths_refer_to_same_file(path, &project)) {
        merge_project_file(&mut merged, &project, trust_project_config).await?;
    }
    if let Some(explicit) = explicit {
        merge_file(&mut merged, explicit, true).await?;
    }
    let mut config: AppConfig = merged
        .try_into()
        .into_diagnostic()
        .wrap_err("configuration contains invalid or unknown fields")?;
    apply_environment(&mut config)?;
    Ok((config, explicit.map_or(global, Path::to_path_buf)))
}

async fn merge_project_file(
    base: &mut toml::Value,
    path: &Path,
    trusted: bool,
) -> miette::Result<()> {
    if trusted {
        return merge_file(base, path, false).await;
    }
    let contents = match tokio::fs::read_to_string(path).await {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error)
                .into_diagnostic()
                .wrap_err_with(|| format!("could not read project config `{}`", path.display()));
        }
    };
    merge_untrusted_project_contents(base, &contents, path)
}

fn merge_untrusted_project_contents(
    base: &mut toml::Value,
    contents: &str,
    path: &Path,
) -> miette::Result<()> {
    let config: UntrustedProjectConfig = toml::from_str(contents).into_diagnostic().wrap_err_with(
        || {
            format!(
                "project config `{}` is untrusted; without --trust-project-config it may only set \
                 `general.default_target` and `general.context_budget`",
                path.display()
            )
        },
    )?;
    let mut general = toml::map::Map::new();
    if let Some(default_target) = config.general.default_target {
        general.insert(
            "default_target".to_owned(),
            toml::Value::String(
                match default_target {
                    AgentName::Claude => "claude",
                    AgentName::Codex => "codex",
                    AgentName::Opencode => "opencode",
                }
                .to_owned(),
            ),
        );
    }
    if let Some(context_budget) = config.general.context_budget {
        let context_budget = i64::try_from(context_budget)
            .into_diagnostic()
            .wrap_err("project context budget exceeds TOML's supported integer range")?;
        general.insert(
            "context_budget".to_owned(),
            toml::Value::Integer(context_budget),
        );
    }
    if !general.is_empty() {
        let mut overlay = toml::map::Map::new();
        overlay.insert("general".to_owned(), toml::Value::Table(general));
        merge(base, toml::Value::Table(overlay));
    }
    Ok(())
}

fn paths_refer_to_same_file(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

pub async fn execute(app: &App, action: ConfigAction) -> miette::Result<i32> {
    match action {
        ConfigAction::Show => {
            if app.json {
                print_json(&app.config)?;
            } else {
                let rendered = toml::to_string_pretty(&app.config)
                    .into_diagnostic()
                    .wrap_err("could not render configuration")?;
                println!("{}", terminal_safe(&rendered));
            }
        }
        ConfigAction::Path => {
            if app.json {
                print_json(&serde_json::json!({ "path": app.config_path }))?;
            } else {
                println!(
                    "{}",
                    terminal_single_line(&app.config_path.display().to_string())
                );
            }
        }
        ConfigAction::Set { key, value } => {
            let mut config = app.config.clone();
            set_value(&mut config, &key, &value)?;
            save_config_key(&app.config_path, &key, &value).await?;
            if app.json {
                print_json(&serde_json::json!({
                    "updated": key,
                    "path": app.config_path,
                }))?;
            } else {
                println!(
                    "Updated {} in {}",
                    terminal_single_line(&key),
                    terminal_single_line(&app.config_path.display().to_string())
                );
            }
        }
    }
    Ok(0)
}

async fn merge_file(base: &mut toml::Value, path: &Path, required: bool) -> miette::Result<()> {
    let contents = match tokio::fs::read_to_string(path).await {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !required => return Ok(()),
        Err(error) => {
            return Err(error)
                .into_diagnostic()
                .wrap_err_with(|| format!("could not read config `{}`", path.display()));
        }
    };
    let overlay: toml::Value = toml::from_str(&contents)
        .into_diagnostic()
        .wrap_err_with(|| format!("could not parse config `{}`", path.display()))?;
    merge(base, overlay);
    Ok(())
}

fn merge(base: &mut toml::Value, overlay: toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(base), toml::Value::Table(overlay)) => {
            for (key, value) in overlay {
                if let Some(existing) = base.get_mut(&key) {
                    merge(existing, value);
                } else {
                    base.insert(key, value);
                }
            }
        }
        (base, overlay) => *base = overlay,
    }
}

fn apply_environment(config: &mut AppConfig) -> miette::Result<()> {
    if let Some(value) = std::env::var_os("CB_DATA_DIR") {
        config.storage.data_dir = value.into();
    }
    if let Ok(value) = std::env::var("CB_CONTEXT_BUDGET") {
        config.general.context_budget = value
            .parse()
            .into_diagnostic()
            .wrap_err("CB_CONTEXT_BUDGET must be a positive integer")?;
    }
    if let Ok(value) = std::env::var("CB_PREVIEW_BEFORE_HANDOFF") {
        config.general.preview_before_handoff = parse_bool(&value)?;
    }
    for (name, destination) in [
        (
            "CB_AGENT_CLAUDE_EXECUTABLE",
            &mut config.agents.claude.executable,
        ),
        (
            "CB_AGENT_CODEX_EXECUTABLE",
            &mut config.agents.codex.executable,
        ),
        (
            "CB_AGENT_OPENCODE_EXECUTABLE",
            &mut config.agents.opencode.executable,
        ),
    ] {
        if let Some(value) = std::env::var_os(name) {
            *destination = value.into();
        }
    }
    config.test_mode = std::env::var("CB_TEST_MODE")
        .ok()
        .map(|value| parse_bool(&value))
        .transpose()?
        .unwrap_or(false);
    if config.general.context_budget == 0 {
        return Err(miette!("general.context_budget must be greater than zero"));
    }
    Ok(())
}

fn set_value(config: &mut AppConfig, key: &str, value: &str) -> miette::Result<()> {
    match key {
        "security.redaction" => {
            config.security.redaction = match value {
                "off" => RedactionMode::Off,
                "standard" => RedactionMode::Standard,
                "strict" => RedactionMode::Strict,
                _ => {
                    return Err(miette!(
                        "security.redaction expects off, standard, or strict"
                    ));
                }
            };
        }
        "general.context_budget" => {
            config.general.context_budget = value
                .parse()
                .into_diagnostic()
                .wrap_err("general.context_budget expects a positive integer")?;
            if config.general.context_budget == 0 {
                return Err(miette!("general.context_budget must be greater than zero"));
            }
        }
        "general.preview_before_handoff" => {
            config.general.preview_before_handoff = parse_bool(value)?;
        }
        "agents.claude.executable" => config.agents.claude.executable = value.into(),
        "agents.codex.executable" => config.agents.codex.executable = value.into(),
        "agents.opencode.executable" => config.agents.opencode.executable = value.into(),
        _ => {
            return Err(miette!(
                "unsupported config key `{key}`; use `cb config show` to inspect supported fields"
            ));
        }
    }
    Ok(())
}

async fn save_config_key(path: &Path, key: &str, value: &str) -> miette::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| miette!("config path `{}` has no parent", path.display()))?;
    super::ensure_private_directory(parent, "configuration directory")?;
    let mut document = match tokio::fs::read_to_string(path).await {
        Ok(contents) => toml::from_str::<toml::Value>(&contents)
            .into_diagnostic()
            .wrap_err_with(|| format!("could not parse existing config `{}`", path.display()))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            toml::Value::Table(toml::map::Map::new())
        }
        Err(error) => {
            return Err(error)
                .into_diagnostic()
                .wrap_err_with(|| format!("could not read config `{}`", path.display()));
        }
    };
    let value = match key {
        "general.context_budget" => toml::Value::Integer(
            value
                .parse()
                .into_diagnostic()
                .wrap_err("context budget must be an integer")?,
        ),
        "general.preview_before_handoff" => toml::Value::Boolean(parse_bool(value)?),
        _ => toml::Value::String(value.to_owned()),
    };
    set_document_value(&mut document, key, value)?;
    let temporary = path.with_extension("toml.tmp");
    tokio::fs::write(
        &temporary,
        toml::to_string_pretty(&document)
            .into_diagnostic()
            .wrap_err("could not serialize configuration")?,
    )
    .await
    .into_diagnostic()
    .wrap_err_with(|| format!("could not write `{}`", temporary.display()))?;
    set_permissions(&temporary, false)?;
    tokio::fs::rename(&temporary, path)
        .await
        .into_diagnostic()
        .wrap_err_with(|| format!("could not atomically replace `{}`", path.display()))?;
    Ok(())
}

fn set_document_value(
    document: &mut toml::Value,
    key: &str,
    value: toml::Value,
) -> miette::Result<()> {
    let (section, field) = key
        .split_once('.')
        .ok_or_else(|| miette!("configuration key `{key}` must contain a section"))?;
    let table = document
        .as_table_mut()
        .ok_or_else(|| miette!("configuration root must be a table"))?;
    let section = table
        .entry(section)
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .ok_or_else(|| miette!("configuration section for `{key}` must be a table"))?;
    section.insert(field.to_owned(), value);
    Ok(())
}

fn parse_bool(value: &str) -> miette::Result<bool> {
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(miette!("expected a boolean, got `{value}`")),
    }
}

fn expand_home(path: &Path) -> miette::Result<PathBuf> {
    let value = path.to_string_lossy();
    if value == "~" || value.starts_with("~/") {
        let home = std::env::var_os("HOME")
            .ok_or_else(|| miette!("HOME is not set; cannot expand `{value}`"))?;
        return Ok(PathBuf::from(home).join(value.trim_start_matches("~/")));
    }
    Ok(path.to_path_buf())
}

fn default_data_dir() -> PathBuf {
    ProjectDirs::from("dev", "context-bridge", "context-bridge").map_or_else(
        || PathBuf::from(".context-bridge-data"),
        |dirs| dirs.data_dir().to_path_buf(),
    )
}

fn default_config_path() -> PathBuf {
    ProjectDirs::from("dev", "context-bridge", "context-bridge").map_or_else(
        || PathBuf::from(".context-bridge.toml"),
        |dirs| dirs.config_dir().join("config.toml"),
    )
}

#[cfg(unix)]
fn set_permissions(path: &Path, directory: bool) -> miette::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = if directory { 0o700 } else { 0o600 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .into_diagnostic()
        .wrap_err_with(|| format!("could not set permissions on `{}`", path.display()))
}

#[cfg(not(unix))]
fn set_permissions(_path: &Path, _directory: bool) -> miette::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{AppConfig, merge_untrusted_project_contents};

    #[test]
    fn untrusted_project_config_merges_only_safe_general_values() {
        let mut base = toml::Value::try_from(AppConfig::default()).expect("default config TOML");
        merge_untrusted_project_contents(
            &mut base,
            "[general]\ndefault_target = \"codex\"\ncontext_budget = 1234\n",
            Path::new(".context-bridge.toml"),
        )
        .expect("safe project config");
        let config: AppConfig = base.try_into().expect("merged config");
        assert!(matches!(
            config.general.default_target,
            super::AgentName::Codex
        ));
        assert_eq!(config.general.context_budget, 1234);
        assert!(matches!(
            config.security.redaction,
            super::RedactionMode::Strict
        ));
        assert_eq!(
            config.agents.codex.executable,
            std::path::PathBuf::from("codex")
        );
    }

    #[test]
    fn untrusted_project_config_rejects_privileged_settings() {
        for contents in [
            "[storage]\ndata_dir = \"/tmp\"\n",
            "[security]\nredaction = \"off\"\n",
            "[agents.codex]\nexecutable = \"./malicious\"\n",
        ] {
            let mut base =
                toml::Value::try_from(AppConfig::default()).expect("default config TOML");
            let error = merge_untrusted_project_contents(
                &mut base,
                contents,
                Path::new(".context-bridge.toml"),
            )
            .expect_err("privileged project setting must require trust");
            assert!(
                error.to_string().contains("--trust-project-config"),
                "unexpected error: {error}"
            );
        }
    }
}
