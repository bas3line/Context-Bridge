use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

use async_trait::async_trait;
use cb_context::{NormalizationContext, RawEvent, normalize_raw_event};
use cb_core::{
    AdapterError, AgentAdapter, AgentCapabilities, AgentInstallation, AgentKind, DiscoveredSession,
    EventSink, ExternalSessionId, ExternalSessionLink, ImportSessionRequest, ImportedSession,
    LaunchRequest, NewContextEvent, PostExitCaptureFailure, PostExitCaptureStage, ProjectContext,
    RefreshResult, ResumeRequest, RunningAgent,
};
use cb_process::{ProcessSpec, resolve_executable, run_attached};
use cb_security::LocalSecretScanner;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::DocumentedCliAdapter;

#[derive(Debug, Clone)]
pub struct CliAdapter {
    kind: AgentKind,
    executable: PathBuf,
    test_mode: bool,
}

impl CliAdapter {
    #[must_use]
    pub fn new(kind: AgentKind, executable: PathBuf, test_mode: bool) -> Self {
        Self {
            kind,
            executable,
            test_mode,
        }
    }

    async fn launch_inner(
        &self,
        bridge_session_id: cb_core::BridgeSessionId,
        project_root: PathBuf,
        bootstrap: Option<String>,
        event_sink_path: PathBuf,
        session_metadata_path: PathBuf,
        resume_id: Option<&ExternalSessionId>,
    ) -> Result<RunningAgent, AdapterError> {
        let installation = self.detect().await?;
        if bootstrap.is_some() && !self.test_mode {
            return Err(AdapterError::UnsupportedVersion {
                agent: self.kind,
                details: "Phase 1 only injects handoffs through the documented fake-adapter \
                          protocol; no unverified prompt flag will be guessed"
                    .to_owned(),
            });
        }
        let mut spec = ProcessSpec::new(installation.executable, project_root);
        if self.test_mode {
            spec.environment.insert(
                "CB_BRIDGE_SESSION_ID".to_owned(),
                bridge_session_id.to_string(),
            );
            spec.environment.insert(
                "CB_EVENT_SINK".to_owned(),
                event_sink_path.to_string_lossy().to_string(),
            );
            spec.environment.insert(
                "CB_SESSION_METADATA".to_owned(),
                session_metadata_path.to_string_lossy().to_string(),
            );
            if let Some(resume_id) = resume_id {
                spec.environment.insert(
                    "CB_RESUME_EXTERNAL_SESSION_ID".to_owned(),
                    resume_id.to_string(),
                );
            }
            if let Some(bootstrap) = bootstrap {
                let bootstrap_path = event_sink_path.with_extension("handoff.md");
                tokio::fs::write(&bootstrap_path, bootstrap)
                    .await
                    .map_err(|source| AdapterError::Launch {
                        agent: self.kind,
                        source,
                    })?;
                spec.environment.insert(
                    "CB_BOOTSTRAP_PATH".to_owned(),
                    bootstrap_path.to_string_lossy().to_string(),
                );
            }
        }
        let outcome = run_attached(&spec)
            .await
            .map_err(|error| AdapterError::Other(error.to_string()))?;
        if !self.test_mode {
            return Ok(RunningAgent {
                exit_code: outcome.exit_code,
                external_session_id: None,
                parser_version: "fake-jsonl-v1".to_owned(),
                events: Vec::new(),
                post_exit_capture_failures: Vec::new(),
            });
        }
        let mut post_exit_capture_failures = Vec::new();
        let external_session_id = match read_session_metadata(&session_metadata_path).await {
            Ok(external_session_id) => external_session_id,
            Err(error) => {
                post_exit_capture_failures.push(PostExitCaptureFailure {
                    stage: PostExitCaptureStage::SessionMetadata,
                    details: error.to_string(),
                });
                None
            }
        };
        let namespace = external_session_id
            .as_ref()
            .map_or_else(|| bridge_session_id.to_string(), ToString::to_string);
        let events = match parse_event_file(&event_sink_path, self.kind, &namespace).await {
            Ok(events) => events,
            Err(error) => {
                post_exit_capture_failures.push(PostExitCaptureFailure {
                    stage: PostExitCaptureStage::EventSink,
                    details: error.to_string(),
                });
                Vec::new()
            }
        };
        Ok(RunningAgent {
            exit_code: outcome.exit_code,
            external_session_id,
            parser_version: "fake-jsonl-v1".to_owned(),
            events,
            post_exit_capture_failures,
        })
    }

    fn fake_session_path(&self, external_id: &ExternalSessionId) -> Option<PathBuf> {
        let base = std::env::var_os("CB_FAKE_SESSIONS_DIR")?;
        Some(
            PathBuf::from(base)
                .join(self.kind.to_string())
                .join(format!("{external_id}.jsonl")),
        )
    }
}

#[async_trait]
impl AgentAdapter for CliAdapter {
    fn kind(&self) -> AgentKind {
        self.kind
    }

    async fn detect(&self) -> Result<AgentInstallation, AdapterError> {
        let Some(executable) = resolve_executable(&self.executable) else {
            return Err(AdapterError::ExecutableNotFound {
                agent: self.kind,
                path: self.executable.clone(),
            });
        };
        let version = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            Command::new(&executable).arg("--version").output(),
        )
        .await
        .ok()
        .and_then(Result::ok)
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|value| !value.is_empty());
        Ok(AgentInstallation {
            executable,
            version,
            compatibility_profile: if self.test_mode {
                "fake-jsonl-v1"
            } else {
                "phase1-launch-only"
            }
            .to_owned(),
        })
    }

    async fn capabilities(&self) -> Result<AgentCapabilities, AdapterError> {
        Ok(if self.test_mode {
            AgentCapabilities {
                session_discovery: true,
                session_import: true,
                native_resume: true,
                initial_prompt_argument: false,
                stdin_prompt: false,
                structured_export: true,
                lifecycle_hooks: false,
                server_api: false,
                interactive_pty: true,
            }
        } else {
            AgentCapabilities {
                interactive_pty: true,
                ..AgentCapabilities::default()
            }
        })
    }

    async fn discover_sessions(
        &self,
        project: Option<&ProjectContext>,
    ) -> Result<Vec<DiscoveredSession>, AdapterError> {
        if !self.test_mode {
            return Ok(Vec::new());
        }
        let Some(base) = std::env::var_os("CB_FAKE_SESSIONS_DIR") else {
            return Ok(Vec::new());
        };
        let directory = PathBuf::from(base).join(self.kind.to_string());
        let mut discovered = Vec::new();
        let Ok(mut entries) = tokio::fs::read_dir(&directory).await else {
            return Ok(Vec::new());
        };
        while let Some(entry) =
            entries
                .next_entry()
                .await
                .map_err(|source| AdapterError::MalformedSession {
                    path: directory.clone(),
                    details: source.to_string(),
                })?
        {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
                continue;
            };
            let external_session_id = ExternalSessionId::new(stem.to_owned())
                .map_err(|error| AdapterError::Other(error.to_string()))?;
            let contents = tokio::fs::read_to_string(&path).await.unwrap_or_default();
            let valid: Vec<RawEvent> = contents
                .lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect();
            let first_user_request = valid.iter().find_map(|event| {
                if event.kind == cb_core::ContextEventKind::UserMessage {
                    match &event.payload {
                        cb_core::ContextEventPayload::Message { content } => Some(content.clone()),
                        _ => None,
                    }
                } else {
                    None
                }
            });
            let modified = entry
                .metadata()
                .await
                .ok()
                .and_then(|metadata| metadata.modified().ok())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            discovered.push(DiscoveredSession {
                agent: self.kind,
                external_session_id,
                project_path: project.map(|project| project.root.clone()),
                updated_at: DateTime::<Utc>::from(modified),
                first_user_request,
                approximate_event_count: valid.len(),
                already_imported: false,
                source_path: Some(path),
            });
        }
        discovered.sort_by_key(|session| std::cmp::Reverse(session.updated_at));
        Ok(discovered)
    }

    async fn import_session(
        &self,
        request: ImportSessionRequest,
        sink: &mut dyn EventSink,
    ) -> Result<ImportedSession, AdapterError> {
        let source_path = request
            .source_path
            .or_else(|| self.fake_session_path(&request.external_session_id))
            .ok_or_else(|| AdapterError::Other("no safe import source is available".to_owned()))?;
        let events = parse_event_file(
            &source_path,
            self.kind,
            request.external_session_id.as_str(),
        )
        .await?;
        for event in &events {
            sink.push(event.clone()).await?;
        }
        Ok(ImportedSession {
            external_session_id: request.external_session_id,
            events,
        })
    }

    async fn launch(&self, request: LaunchRequest) -> Result<RunningAgent, AdapterError> {
        self.launch_inner(
            request.bridge_session_id,
            request.project_root,
            request.bootstrap,
            request.event_sink_path,
            request.session_metadata_path,
            None,
        )
        .await
    }

    async fn resume(&self, request: ResumeRequest) -> Result<RunningAgent, AdapterError> {
        if !self.test_mode {
            return Err(AdapterError::UnsupportedVersion {
                agent: self.kind,
                details: "native resume is not enabled for this compatibility profile".to_owned(),
            });
        }
        self.launch_inner(
            request.bridge_session_id,
            request.project_root,
            Some(request.bootstrap),
            request.event_sink_path,
            request.session_metadata_path,
            Some(&request.external_session_id),
        )
        .await
    }

    async fn refresh(
        &self,
        link: &ExternalSessionLink,
        sink: &mut dyn EventSink,
    ) -> Result<RefreshResult, AdapterError> {
        let Some(path) = link
            .source_path
            .clone()
            .or_else(|| self.fake_session_path(&link.external_session_id))
        else {
            return Ok(RefreshResult { events: Vec::new() });
        };
        let events = parse_event_file(&path, self.kind, link.external_session_id.as_str()).await?;
        for event in &events {
            sink.push(event.clone()).await?;
        }
        Ok(RefreshResult { events })
    }
}

#[derive(Default)]
pub struct AdapterRegistry {
    adapters: BTreeMap<AgentKind, Arc<dyn AgentAdapter>>,
}

impl AdapterRegistry {
    #[must_use]
    pub fn standard(overrides: &BTreeMap<AgentKind, PathBuf>, test_mode: bool) -> Self {
        let mut adapters: BTreeMap<AgentKind, Arc<dyn AgentAdapter>> = BTreeMap::new();
        for kind in [AgentKind::Codex, AgentKind::ClaudeCode, AgentKind::OpenCode] {
            let executable = overrides
                .get(&kind)
                .cloned()
                .unwrap_or_else(|| PathBuf::from(kind.executable_name()));
            let adapter: Arc<dyn AgentAdapter> = if test_mode {
                Arc::new(CliAdapter::new(kind, executable, true))
            } else {
                Arc::new(DocumentedCliAdapter::new(kind, executable))
            };
            adapters.insert(kind, adapter);
        }
        Self { adapters }
    }

    pub fn get(&self, kind: AgentKind) -> Result<Arc<dyn AgentAdapter>, AdapterError> {
        self.adapters
            .get(&kind)
            .cloned()
            .ok_or_else(|| AdapterError::Other(format!("no adapter registered for {kind}")))
    }
}

async fn parse_event_file(
    path: &Path,
    agent: AgentKind,
    namespace: &str,
) -> Result<Vec<NewContextEvent>, AdapterError> {
    let contents = match tokio::fs::read_to_string(path).await {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(AdapterError::MalformedSession {
                path: path.to_path_buf(),
                details: error.to_string(),
            });
        }
    };
    let scanner = LocalSecretScanner::default();
    let parser_name = format!("fake-jsonl:{namespace}");
    let context = NormalizationContext {
        agent,
        parser_name: &parser_name,
        parser_version: "1",
        external_session_namespace: namespace,
        source_path: Some(path),
    };
    let mut events = Vec::new();
    let mut invalid = 0_usize;
    for (index, line) in contents.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<RawEvent>(line) {
            Ok(raw) => events.push(normalize_raw_event(raw, context, index, &scanner)),
            Err(error) => {
                invalid += 1;
                tracing::warn!(path = %path.display(), line = index + 1, %error, "skipping malformed external event");
            }
        }
    }
    if events.is_empty() && invalid > 0 {
        return Err(AdapterError::MalformedSession {
            path: path.to_path_buf(),
            details: format!("all {invalid} records were malformed"),
        });
    }
    Ok(events)
}

#[derive(Debug, Serialize, Deserialize)]
struct SessionMetadata {
    external_session_id: String,
}

async fn read_session_metadata(path: &Path) -> Result<Option<ExternalSessionId>, AdapterError> {
    let contents = match tokio::fs::read_to_string(path).await {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(AdapterError::MalformedSession {
                path: path.to_path_buf(),
                details: error.to_string(),
            });
        }
    };
    let metadata: SessionMetadata =
        serde_json::from_str(&contents).map_err(|error| AdapterError::MalformedSession {
            path: path.to_path_buf(),
            details: error.to_string(),
        })?;
    ExternalSessionId::new(metadata.external_session_id)
        .map(Some)
        .map_err(|error| AdapterError::MalformedSession {
            path: path.to_path_buf(),
            details: error.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use cb_core::AgentKind;

    use super::parse_event_file;

    #[tokio::test]
    async fn supported_fake_fixture_versions_normalize_without_panicking() {
        for (agent, directory) in [
            (AgentKind::ClaudeCode, "claude"),
            (AgentKind::OpenCode, "opencode"),
            (AgentKind::Codex, "codex"),
        ] {
            for version in ["v1.0", "v1.1"] {
                let path = fixture(directory, version);
                let events = parse_event_file(&path, agent, version)
                    .await
                    .expect("supported fixture should normalize");
                assert!(!events.is_empty());
                assert!(events.iter().all(|event| event.source_agent == Some(agent)));
                if version == "v1.1" {
                    assert!(
                        events[0].metadata.get("unknown_optional").is_some(),
                        "versioned metadata must be preserved"
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn malformed_fixtures_fail_safely() {
        for (agent, directory) in [
            (AgentKind::ClaudeCode, "claude"),
            (AgentKind::OpenCode, "opencode"),
            (AgentKind::Codex, "codex"),
        ] {
            let result =
                parse_event_file(&fixture(directory, "malformed"), agent, "malformed").await;
            assert!(result.is_err());
        }
    }

    fn fixture(agent: &str, version: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(agent)
            .join(version)
            .join("session.jsonl")
    }
}
