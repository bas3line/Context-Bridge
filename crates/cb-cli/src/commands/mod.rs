mod checkpoint;
mod config;
mod continue_cmd;
mod diff;
mod doctor;
mod export;
mod import;
mod integrate;
mod run;
mod sessions;
mod show;
mod timeline;

use std::{
    collections::BTreeMap,
    io::ErrorKind,
    path::{Path, PathBuf},
    str::FromStr,
};

use cb_adapters::AdapterRegistry;
use cb_core::{
    AgentKind, BridgeSession, BridgeSessionId, ProjectId, ProjectRecord, SessionRepository,
};
use cb_project::{ResolvedProject, project_id, resolve_project_root};
use cb_storage::SqliteStore;
use chrono::Utc;
use clap::{ArgAction, Parser, Subcommand, ValueEnum};
use miette::{IntoDiagnostic, WrapErr, miette};

pub use config::{AppConfig, ConfigAction};

#[derive(Debug, Parser)]
#[command(
    name = "cb",
    version,
    about = "Local-first context handoffs between AI coding agents",
    long_about = None
)]
pub struct Cli {
    #[arg(long, global = true, value_name = "PATH")]
    pub project: Option<PathBuf>,
    #[arg(long, global = true, env = "CB_DATA_DIR", value_name = "PATH")]
    pub data_dir: Option<PathBuf>,
    #[arg(long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,
    /// Allow repository-local configuration to change storage, security, and executable settings.
    #[arg(long, global = true)]
    pub trust_project_config: bool,
    #[arg(long, global = true)]
    pub json: bool,
    #[arg(long, global = true, action = ArgAction::Count)]
    pub verbose: u8,
    #[arg(long, global = true)]
    pub quiet: bool,
    #[arg(long, global = true)]
    pub no_color: bool,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Launch an agent and record its observable session context.
    Run {
        #[arg(value_parser = parse_agent)]
        agent: AgentKind,
    },
    /// Continue a canonical session in another agent.
    Continue {
        #[arg(long, value_parser = parse_agent)]
        from: Option<AgentKind>,
        #[arg(long, conflicts_with = "last")]
        session: Option<cb_core::BridgeSessionId>,
        #[arg(long, conflicts_with = "session")]
        last: bool,
        #[arg(long, value_parser = parse_agent)]
        to: AgentKind,
        #[arg(long)]
        budget: Option<usize>,
        #[arg(long)]
        preview: bool,
    },
    /// Import an existing agent session without altering it.
    Import {
        #[arg(value_parser = parse_agent)]
        agent: AgentKind,
        #[arg(long)]
        session: Option<String>,
        /// Import the documented raw export. It can include local secrets, so this is opt-in.
        #[arg(long)]
        full: bool,
    },
    /// List canonical bridge sessions.
    Sessions,
    /// Show a canonical session and its external links.
    Show {
        session_id: cb_core::BridgeSessionId,
    },
    /// Show the ordered canonical event timeline.
    Timeline {
        session_id: cb_core::BridgeSessionId,
        #[arg(long)]
        include_sensitive: bool,
    },
    /// Show the latest captured Git diff.
    Diff {
        session_id: cb_core::BridgeSessionId,
    },
    /// Add a manual checkpoint to the current project session.
    Checkpoint {
        #[arg(long)]
        note: Option<String>,
    },
    /// Export canonical events and derived context.
    Export {
        session_id: cb_core::BridgeSessionId,
        #[arg(long, value_enum, default_value_t = ExportFormat::Markdown)]
        format: ExportFormat,
        #[arg(long)]
        redacted: bool,
    },
    /// Manage opt-in vendor integrations.
    Integrate {
        #[arg(value_parser = parse_agent)]
        agent: AgentKind,
        #[arg(long)]
        remove: bool,
    },
    /// Diagnose the local installation and compatibility profiles.
    Doctor,
    /// Inspect or change Context Bridge configuration.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ExportFormat {
    Markdown,
    Json,
}

pub struct App {
    pub json: bool,
    pub quiet: bool,
    pub config: AppConfig,
    pub config_path: PathBuf,
    pub data_dir: PathBuf,
    pub project: ResolvedProject,
    pub project_id: ProjectId,
    pub store: SqliteStore,
    pub adapters: AdapterRegistry,
}

impl App {
    pub async fn require_current_project_session(
        &self,
        session_id: BridgeSessionId,
    ) -> miette::Result<BridgeSession> {
        let session = self
            .store
            .get_session(session_id)
            .await
            .into_diagnostic()?
            .ok_or_else(|| miette!("bridge session `{session_id}` was not found"))?;
        if session.project_id != self.project_id {
            let root = self
                .store
                .project(&session.project_id)
                .await
                .into_diagnostic()?
                .map_or_else(
                    || "<unknown>".to_owned(),
                    |project| project.root.display().to_string(),
                );
            return Err(miette!(
                "bridge session `{session_id}` belongs to `{root}`, not the current project. \
                 Retry with `--project {root}`."
            ));
        }
        Ok(session)
    }

    pub fn scanner(&self) -> cb_security::LocalSecretScanner {
        let level = match self.config.security.redaction {
            config::RedactionMode::Off => cb_security::RedactionLevel::Off,
            config::RedactionMode::Standard => cb_security::RedactionLevel::Standard,
            config::RedactionMode::Strict => cb_security::RedactionLevel::Strict,
        };
        cb_security::LocalSecretScanner::new(level)
    }

    pub fn path_policy(&self) -> miette::Result<cb_security::PathPolicy> {
        let mut policy = cb_security::PathPolicy::new(&self.config.security.excluded_paths)
            .into_diagnostic()
            .wrap_err("could not compile security.excluded_paths")?;
        if let Ok(relative) = self.data_dir.strip_prefix(&self.project.root) {
            policy = policy.with_excluded_root(relative.to_path_buf());
        }
        Ok(policy)
    }

    pub async fn snapshot(&self) -> miette::Result<cb_project::ProjectSnapshot> {
        let policy = self.path_policy()?;
        cb_project::capture_project_snapshot_with_policy(&self.project.root, &policy)
            .await
            .into_diagnostic()
            .wrap_err("could not capture project state")
    }
}

pub async fn execute(cli: Cli) -> miette::Result<i32> {
    let doctor_verbose = cli.verbose > 0;
    let cwd = std::env::current_dir()
        .into_diagnostic()
        .wrap_err("could not determine the current directory")?;
    let project_hint = cli.project.clone().unwrap_or(cwd);
    let project = resolve_project_root(&project_hint)
        .await
        .into_diagnostic()
        .wrap_err_with(|| {
            format!(
                "could not resolve a project from `{}`",
                project_hint.display()
            )
        })?;
    let (mut config, config_path) = config::load_config(
        &project.root,
        cli.config.as_deref(),
        cli.trust_project_config,
    )
    .await?;
    if let Some(data_dir) = cli.data_dir {
        config.storage.data_dir = data_dir;
    }
    let configured_data_dir = config.expanded_data_dir()?;
    let data_dir = if configured_data_dir.is_absolute() {
        configured_data_dir
    } else {
        project.root.join(configured_data_dir)
    };
    ensure_private_directory(&data_dir, "data directory")?;
    let data_dir = data_dir
        .canonicalize()
        .into_diagnostic()
        .wrap_err_with(|| format!("could not canonicalize `{}`", data_dir.display()))?;
    if data_dir == project.root {
        return Err(miette!(
            "the Context Bridge data directory cannot be the project root; \
             choose a dedicated directory with `--data-dir` or CB_DATA_DIR"
        ));
    }
    let store = SqliteStore::open(&data_dir.join("context-bridge.db"))
        .await
        .into_diagnostic()
        .wrap_err("could not open the Context Bridge database")?;
    let project_id = project_id(&project);
    let now = Utc::now();
    store
        .upsert_project(&ProjectRecord {
            id: project_id.clone(),
            root: project.root.clone(),
            is_git: project.is_git,
            created_at: now,
            updated_at: now,
        })
        .await
        .into_diagnostic()?;

    let overrides = agent_overrides(&config);
    let adapters = AdapterRegistry::standard(&overrides, config.test_mode);
    let app = App {
        json: cli.json,
        quiet: cli.quiet,
        config,
        config_path,
        data_dir,
        project,
        project_id,
        store,
        adapters,
    };
    match cli.command {
        Command::Run { agent } => run::execute(&app, agent).await,
        Command::Continue {
            from,
            session,
            last,
            to,
            budget,
            preview,
        } => continue_cmd::execute(&app, from, session, last, to, budget, preview).await,
        Command::Import {
            agent,
            session,
            full,
        } => import::execute(&app, agent, session, full).await,
        Command::Sessions => sessions::execute(&app).await,
        Command::Show { session_id } => show::execute(&app, session_id).await,
        Command::Timeline {
            session_id,
            include_sensitive,
        } => timeline::execute(&app, session_id, include_sensitive).await,
        Command::Diff { session_id } => diff::execute(&app, session_id).await,
        Command::Checkpoint { note } => checkpoint::execute(&app, note).await,
        Command::Export {
            session_id,
            format,
            redacted,
        } => export::execute(&app, session_id, format, redacted).await,
        Command::Integrate { agent, remove } => integrate::execute(&app, agent, remove).await,
        Command::Doctor => doctor::execute(&app, doctor_verbose).await,
        Command::Config { action } => config::execute(&app, action).await,
    }
}

fn agent_overrides(config: &AppConfig) -> BTreeMap<AgentKind, PathBuf> {
    [
        (AgentKind::Codex, &config.agents.codex.executable),
        (AgentKind::ClaudeCode, &config.agents.claude.executable),
        (AgentKind::OpenCode, &config.agents.opencode.executable),
    ]
    .into_iter()
    .map(|(kind, path)| (kind, path.clone()))
    .collect()
}

fn parse_agent(value: &str) -> Result<AgentKind, String> {
    AgentKind::from_str(value).map_err(|error| error.to_string())
}

pub(crate) fn ensure_private_directory(path: &Path, purpose: &str) -> miette::Result<()> {
    let mut missing = Vec::new();
    let mut cursor = path;
    loop {
        match std::fs::symlink_metadata(cursor) {
            Ok(metadata) => {
                if !metadata.file_type().is_dir() {
                    return Err(miette!(
                        "{purpose} `{}` must be a directory, not a file or symlink",
                        cursor.display(),
                    ));
                }
                break;
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                missing.push(cursor.to_path_buf());
                cursor = cursor.parent().ok_or_else(|| {
                    miette!("{purpose} `{}` has no existing parent", path.display())
                })?;
            }
            Err(error) => {
                return Err(error).into_diagnostic().wrap_err_with(|| {
                    format!("could not inspect {purpose} `{}`", cursor.display())
                });
            }
        }
    }
    for directory in missing.iter().rev() {
        match create_private_directory(directory) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                validate_private_directory(directory, purpose)?;
            }
            Err(error) => {
                return Err(error).into_diagnostic().wrap_err_with(|| {
                    format!("could not create {purpose} `{}`", directory.display())
                });
            }
        }
    }
    validate_private_directory(path, purpose)
}

#[cfg(unix)]
fn create_private_directory(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    let mut builder = std::fs::DirBuilder::new();
    builder.mode(0o700).create(path)
}

#[cfg(not(unix))]
fn create_private_directory(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir(path)
}

fn validate_private_directory(path: &Path, purpose: &str) -> miette::Result<()> {
    let metadata = std::fs::symlink_metadata(path)
        .into_diagnostic()
        .wrap_err_with(|| format!("could not inspect {purpose} `{}`", path.display()))?;
    if !metadata.file_type().is_dir() {
        return Err(miette!(
            "{purpose} `{}` must be a directory, not a file or symlink",
            path.display(),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(miette!(
                "refusing to use existing {purpose} `{}` because it is accessible by group or other users; \
                 choose a dedicated private directory instead",
                path.display(),
            ));
        }
    }
    Ok(())
}
