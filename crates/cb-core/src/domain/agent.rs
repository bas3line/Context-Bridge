use std::{fmt, path::PathBuf, str::FromStr};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::ExternalSessionId;

/// A coding agent supported by Context Bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum AgentKind {
    #[serde(rename = "codex")]
    Codex,
    #[serde(rename = "claude")]
    ClaudeCode,
    #[serde(rename = "opencode")]
    OpenCode,
}

impl AgentKind {
    #[must_use]
    pub const fn executable_name(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::ClaudeCode => "claude",
            Self::OpenCode => "opencode",
        }
    }
}

impl fmt::Display for AgentKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Codex => "codex",
            Self::ClaudeCode => "claude",
            Self::OpenCode => "opencode",
        })
    }
}

impl FromStr for AgentKind {
    type Err = ParseAgentKindError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "codex" => Ok(Self::Codex),
            "claude" | "claude-code" => Ok(Self::ClaudeCode),
            "opencode" | "open-code" => Ok(Self::OpenCode),
            _ => Err(ParseAgentKindError(value.to_owned())),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("unsupported agent `{0}`; expected codex, claude, or opencode")]
pub struct ParseAgentKindError(String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AgentCapabilities {
    pub session_discovery: bool,
    pub session_import: bool,
    pub native_resume: bool,
    pub initial_prompt_argument: bool,
    pub stdin_prompt: bool,
    pub structured_export: bool,
    pub lifecycle_hooks: bool,
    pub server_api: bool,
    pub interactive_pty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentInstallation {
    pub executable: PathBuf,
    pub version: Option<String>,
    pub compatibility_profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredSession {
    pub agent: AgentKind,
    pub external_session_id: ExternalSessionId,
    pub project_path: Option<PathBuf>,
    pub updated_at: DateTime<Utc>,
    pub first_user_request: Option<String>,
    pub approximate_event_count: usize,
    pub already_imported: bool,
    pub source_path: Option<PathBuf>,
}
