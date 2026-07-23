use std::{
    fmt,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    AgentCapabilities, AgentInstallation, AgentKind, BridgeSessionId, DiscoveredSession,
    ExternalSessionId, ExternalSessionLink, NewContextEvent,
};

#[derive(Debug, Clone)]
pub struct ProjectContext {
    pub root: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ImportSessionRequest {
    pub bridge_session_id: BridgeSessionId,
    pub external_session_id: ExternalSessionId,
    pub source_path: Option<PathBuf>,
    /// Use the vendor's documented unsanitized export only after explicit user opt-in.
    pub full_context: bool,
}

#[derive(Debug, Clone)]
pub struct LaunchRequest {
    pub bridge_session_id: BridgeSessionId,
    pub project_root: PathBuf,
    pub bootstrap: Option<String>,
    pub event_sink_path: PathBuf,
    pub session_metadata_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ResumeRequest {
    pub bridge_session_id: BridgeSessionId,
    pub external_session_id: ExternalSessionId,
    pub project_root: PathBuf,
    pub bootstrap: String,
    pub event_sink_path: PathBuf,
    pub session_metadata_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunningAgent {
    pub exit_code: i32,
    pub external_session_id: Option<ExternalSessionId>,
    /// Compatibility/parser profile that established this external session link.
    pub parser_version: String,
    pub events: Vec<NewContextEvent>,
    /// Capture failures observed only after the child process exited.
    ///
    /// These must not be returned as adapter launch errors: callers still need
    /// to reconcile the project and finalize the bridge session using the
    /// child's real exit status.
    #[serde(default)]
    pub post_exit_capture_failures: Vec<PostExitCaptureFailure>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PostExitCaptureStage {
    EventSink,
    SessionMetadata,
}

impl fmt::Display for PostExitCaptureStage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EventSink => "event sink",
            Self::SessionMetadata => "session metadata",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostExitCaptureFailure {
    pub stage: PostExitCaptureStage,
    pub details: String,
}

impl fmt::Display for PostExitCaptureFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.stage, self.details)
    }
}

#[derive(Debug, Clone)]
pub struct ImportedSession {
    pub external_session_id: ExternalSessionId,
    pub events: Vec<NewContextEvent>,
}

#[derive(Debug, Clone)]
pub struct RefreshResult {
    pub events: Vec<NewContextEvent>,
}

#[async_trait]
pub trait EventSink: Send {
    async fn push(&mut self, event: NewContextEvent) -> Result<(), AdapterError>;
}

#[async_trait]
pub trait AgentAdapter: Send + Sync {
    fn kind(&self) -> AgentKind;

    async fn detect(&self) -> Result<AgentInstallation, AdapterError>;

    async fn capabilities(&self) -> Result<AgentCapabilities, AdapterError>;

    async fn discover_sessions(
        &self,
        project: Option<&ProjectContext>,
    ) -> Result<Vec<DiscoveredSession>, AdapterError>;

    async fn import_session(
        &self,
        request: ImportSessionRequest,
        sink: &mut dyn EventSink,
    ) -> Result<ImportedSession, AdapterError>;

    async fn launch(&self, request: LaunchRequest) -> Result<RunningAgent, AdapterError>;

    async fn resume(&self, request: ResumeRequest) -> Result<RunningAgent, AdapterError>;

    async fn refresh(
        &self,
        link: &ExternalSessionLink,
        sink: &mut dyn EventSink,
    ) -> Result<RefreshResult, AdapterError>;
}

#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("{agent} executable was not found at `{path}`")]
    ExecutableNotFound { agent: AgentKind, path: PathBuf },
    #[error("{agent} process could not be launched: {source}")]
    Launch {
        agent: AgentKind,
        #[source]
        source: std::io::Error,
    },
    #[error("unsupported {agent} compatibility profile: {details}")]
    UnsupportedVersion { agent: AgentKind, details: String },
    #[error("malformed external session data at `{path}`: {details}")]
    MalformedSession { path: PathBuf, details: String },
    #[error("{0}")]
    Other(String),
}

pub fn path_is_readable(path: &Path) -> bool {
    std::fs::File::open(path).is_ok()
}
