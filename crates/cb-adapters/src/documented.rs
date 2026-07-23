//! Production adapters using only documented command-line interfaces.
//!
//! No adapter here reads vendor-owned storage or scrapes terminal output.

use std::{
    io::Write,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use async_trait::async_trait;
use cb_context::{NormalizationContext, RawEvent, normalize_raw_event};
use cb_core::{
    AdapterError, AgentAdapter, AgentCapabilities, AgentInstallation, AgentKind, ContextEventKind,
    ContextEventPayload, DiscoveredSession, EventSink, ExternalSessionId, ExternalSessionLink,
    ImportSessionRequest, ImportedSession, LaunchRequest, ProjectContext, RefreshResult,
    ResumeRequest, RunningAgent,
};
use cb_process::{ProcessSpec, resolve_executable, run_attached};
use cb_security::LocalSecretScanner;
use chrono::{DateTime, Utc};
use serde_json::Value;
use tempfile::NamedTempFile;
use tokio::process::Command;

const OPENCODE_PROFILE: &str = "opencode-cli-1.18";
const CLAUDE_PROFILE: &str = "claude-cli-2.1";
const CODEX_PROFILE: &str = "codex-cli-0.145";
const OPENCODE_PARSER_VERSION: &str = "opencode-export-v1";
const OPENCODE_FULL_PARSER_VERSION: &str = "opencode-export-full-v2";
const CLAUDE_HANDOFF_PROMPT: &str = "Context Bridge supplied the prior session in your appended system context. Continue the task now; inspect the repository before editing.";

#[derive(Debug, Clone)]
pub struct DocumentedCliAdapter {
    kind: AgentKind,
    executable: PathBuf,
}

impl DocumentedCliAdapter {
    #[must_use]
    pub fn new(kind: AgentKind, executable: PathBuf) -> Self {
        Self { kind, executable }
    }

    async fn installation(&self) -> Result<AgentInstallation, AdapterError> {
        let executable = resolve_executable(&self.executable).ok_or_else(|| {
            AdapterError::ExecutableNotFound {
                agent: self.kind,
                path: self.executable.clone(),
            }
        })?;
        // Agent CLIs can be briefly busy while their interactive UI is live.
        // A transient version-probe timeout must not turn a known profile into
        // an unverified one halfway through an import.
        let mut version = None;
        for attempt in 0..3 {
            let output = tokio::time::timeout(
                Duration::from_secs(5),
                Command::new(&executable).arg("--version").output(),
            )
            .await
            .ok()
            .and_then(Result::ok);
            version = output
                .filter(|output| output.status.success())
                .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
                .filter(|value| !value.is_empty());
            if version.is_some() || attempt == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        Ok(AgentInstallation {
            executable,
            compatibility_profile: profile(self.kind, version.as_deref()),
            version,
        })
    }

    fn unsupported(&self, installation: &AgentInstallation, feature: &str) -> AdapterError {
        AdapterError::UnsupportedVersion {
            agent: self.kind,
            details: format!(
                "profile `{}` does not safely support {feature}; run `cb doctor --verbose` and upgrade the agent before retrying",
                installation.compatibility_profile
            ),
        }
    }

    async fn attached(&self, spec: ProcessSpec) -> Result<i32, AdapterError> {
        run_attached(&spec)
            .await
            .map(|outcome| outcome.exit_code)
            .map_err(|error| AdapterError::Other(error.to_string()))
    }

    async fn opencode_sessions(
        &self,
        project: Option<&Path>,
    ) -> Result<Vec<DiscoveredSession>, AdapterError> {
        let installation = self.installation().await?;
        if installation.compatibility_profile != OPENCODE_PROFILE {
            return Err(self.unsupported(&installation, "structured session discovery"));
        }
        let mut command = Command::new(installation.executable);
        command.args(["session", "list", "--format", "json"]);
        if let Some(project) = project {
            command.current_dir(project);
        }
        let output = command
            .output()
            .await
            .map_err(|source| AdapterError::Launch {
                agent: self.kind,
                source,
            })?;
        if !output.status.success() {
            return Err(AdapterError::Other(format!(
                "opencode session list failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        parse_opencode_sessions(&output.stdout, project)
    }

    async fn opencode_export(
        &self,
        id: &ExternalSessionId,
        full_context: bool,
    ) -> Result<Vec<cb_core::NewContextEvent>, AdapterError> {
        let installation = self.installation().await?;
        if installation.compatibility_profile != OPENCODE_PROFILE {
            return Err(self.unsupported(&installation, "structured session import"));
        }
        // OpenCode 1.18 can produce multi-megabyte exports. Its stdout-pipe
        // path may terminate early for large sessions, so write the documented
        // sanitized export to a private temporary file and read it only after
        // the command has fully exited. We still never read vendor storage.
        let export_file = NamedTempFile::new().map_err(|error| {
            AdapterError::Other(format!("could not create export file: {error}"))
        })?;
        let export_path = export_file.path().to_path_buf();
        let stdout = export_file
            .reopen()
            .map_err(|error| AdapterError::Other(format!("could not open export file: {error}")))?;
        let stderr_file = NamedTempFile::new().map_err(|error| {
            AdapterError::Other(format!("could not create export error file: {error}"))
        })?;
        let stderr_path = stderr_file.path().to_path_buf();
        let stderr = stderr_file.reopen().map_err(|error| {
            AdapterError::Other(format!("could not open export error file: {error}"))
        })?;
        let status = Command::new(installation.executable)
            .args(opencode_export_args(id, full_context))
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .status()
            .await
            .map_err(|source| AdapterError::Launch {
                agent: self.kind,
                source,
            })?;
        if !status.success() {
            let stderr = tokio::fs::read_to_string(&stderr_path)
                .await
                .unwrap_or_default();
            return Err(AdapterError::Other(format!(
                "opencode export `{id}` failed: {}",
                stderr.trim()
            )));
        }
        let bytes = tokio::fs::read(&export_path).await.map_err(|error| {
            AdapterError::Other(format!("could not read sanitized export: {error}"))
        })?;
        parse_opencode_export(
            &bytes,
            id.as_str(),
            if full_context {
                OPENCODE_FULL_PARSER_VERSION
            } else {
                OPENCODE_PARSER_VERSION
            },
        )
    }

    async fn launch_opencode(&self, request: LaunchRequest) -> Result<RunningAgent, AdapterError> {
        let installation = self.installation().await?;
        // Do not block interactive startup on a global session-list request.
        // We attempt best-effort linking only after the user exits OpenCode.
        let launched_at = Utc::now();
        let mut spec = ProcessSpec::new(installation.executable, request.project_root.clone());
        spec.args.push(request.project_root.display().to_string());
        if let Some(bootstrap) = request.bootstrap {
            spec.args.extend(["--prompt".to_owned(), bootstrap]);
        }
        let exit_code = self.attached(spec).await?;
        let external_session_id = self
            .opencode_sessions(Some(&request.project_root))
            .await
            .ok()
            .and_then(|sessions| {
                sessions
                    .into_iter()
                    .filter(|session| {
                        session.project_path.as_deref() == Some(request.project_root.as_path())
                            && session.updated_at >= launched_at
                    })
                    .max_by_key(|session| session.updated_at)
            })
            .map(|session| session.external_session_id);
        Ok(RunningAgent {
            exit_code,
            external_session_id,
            parser_version: OPENCODE_PROFILE.to_owned(),
            events: Vec::new(),
            post_exit_capture_failures: Vec::new(),
        })
    }
}

#[async_trait]
impl AgentAdapter for DocumentedCliAdapter {
    fn kind(&self) -> AgentKind {
        self.kind
    }

    async fn detect(&self) -> Result<AgentInstallation, AdapterError> {
        self.installation().await
    }

    async fn capabilities(&self) -> Result<AgentCapabilities, AdapterError> {
        let installation = self.installation().await?;
        Ok(capabilities(self.kind, &installation.compatibility_profile))
    }

    async fn discover_sessions(
        &self,
        project: Option<&ProjectContext>,
    ) -> Result<Vec<DiscoveredSession>, AdapterError> {
        match self.kind {
            AgentKind::OpenCode => {
                self.opencode_sessions(project.map(|project| project.root.as_path()))
                    .await
            }
            AgentKind::ClaudeCode | AgentKind::Codex => Ok(Vec::new()),
        }
    }

    async fn import_session(
        &self,
        request: ImportSessionRequest,
        sink: &mut dyn EventSink,
    ) -> Result<ImportedSession, AdapterError> {
        if self.kind != AgentKind::OpenCode {
            return Err(self.unsupported(&self.installation().await?, "structured session import"));
        }
        let events = self
            .opencode_export(&request.external_session_id, request.full_context)
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
        let installation = self.installation().await?;
        let capabilities = capabilities(self.kind, &installation.compatibility_profile);
        if request.bootstrap.is_some() && !capabilities.initial_prompt_argument {
            return Err(self.unsupported(&installation, "a verified initial handoff prompt"));
        }
        if self.kind == AgentKind::OpenCode && capabilities.initial_prompt_argument {
            return self.launch_opencode(request).await;
        }
        if !capabilities.initial_prompt_argument {
            // An unknown version may still be opened as a normal interactive
            // shell.  We deliberately do not pass a guessed prompt or resume
            // flag, and do not claim an external session link was created.
            let mut spec = ProcessSpec::new(installation.executable, request.project_root.clone());
            if self.kind == AgentKind::OpenCode {
                spec.args.push(request.project_root.display().to_string());
            }
            let exit_code = self.attached(spec).await?;
            return Ok(RunningAgent {
                exit_code,
                external_session_id: None,
                parser_version: installation.compatibility_profile,
                events: Vec::new(),
                post_exit_capture_failures: Vec::new(),
            });
        }
        let external_session_id = if self.kind == AgentKind::ClaudeCode {
            Some(
                ExternalSessionId::new(request.bridge_session_id.to_string())
                    .map_err(|error| AdapterError::Other(error.to_string()))?,
            )
        } else {
            None
        };
        let mut spec = ProcessSpec::new(installation.executable, request.project_root);
        match self.kind {
            AgentKind::ClaudeCode => {
                let handoff_file =
                    write_claude_handoff(request.bootstrap.as_deref().unwrap_or_default())?;
                spec.args.extend([
                    "--session-id".to_owned(),
                    external_session_id
                        .as_ref()
                        .expect("Claude IDs are present")
                        .to_string(),
                    "--append-system-prompt-file".to_owned(),
                    handoff_file.path().display().to_string(),
                ]);
                spec.args.push(CLAUDE_HANDOFF_PROMPT.to_owned());
                let exit_code = self.attached(spec).await?;
                return Ok(RunningAgent {
                    exit_code,
                    external_session_id,
                    parser_version: installation.compatibility_profile,
                    events: Vec::new(),
                    post_exit_capture_failures: Vec::new(),
                });
            }
            AgentKind::Codex => {
                if let Some(bootstrap) = request.bootstrap {
                    spec.args.push(bootstrap);
                }
            }
            AgentKind::OpenCode => unreachable!("handled above"),
        }
        let exit_code = self.attached(spec).await?;
        Ok(RunningAgent {
            exit_code,
            external_session_id,
            parser_version: installation.compatibility_profile,
            events: Vec::new(),
            post_exit_capture_failures: Vec::new(),
        })
    }

    async fn resume(&self, request: ResumeRequest) -> Result<RunningAgent, AdapterError> {
        let installation = self.installation().await?;
        if !capabilities(self.kind, &installation.compatibility_profile).native_resume {
            return Err(self.unsupported(&installation, "native resume"));
        }
        let root = request.project_root.clone();
        let mut spec = ProcessSpec::new(installation.executable, root.clone());
        match self.kind {
            AgentKind::Codex => spec.args.extend([
                "resume".to_owned(),
                request.external_session_id.to_string(),
                request.bootstrap,
            ]),
            AgentKind::ClaudeCode => {
                let handoff_file = write_claude_handoff(&request.bootstrap)?;
                spec.args.extend([
                    "--resume".to_owned(),
                    request.external_session_id.to_string(),
                    "--append-system-prompt-file".to_owned(),
                    handoff_file.path().display().to_string(),
                    CLAUDE_HANDOFF_PROMPT.to_owned(),
                ]);
                let exit_code = self.attached(spec).await?;
                return Ok(RunningAgent {
                    exit_code,
                    external_session_id: Some(request.external_session_id),
                    parser_version: installation.compatibility_profile,
                    events: Vec::new(),
                    post_exit_capture_failures: Vec::new(),
                });
            }
            AgentKind::OpenCode => spec.args.extend([
                root.display().to_string(),
                "--session".to_owned(),
                request.external_session_id.to_string(),
                "--prompt".to_owned(),
                request.bootstrap,
            ]),
        }
        let exit_code = self.attached(spec).await?;
        Ok(RunningAgent {
            exit_code,
            external_session_id: Some(request.external_session_id),
            parser_version: installation.compatibility_profile,
            events: Vec::new(),
            post_exit_capture_failures: Vec::new(),
        })
    }

    async fn refresh(
        &self,
        link: &ExternalSessionLink,
        sink: &mut dyn EventSink,
    ) -> Result<RefreshResult, AdapterError> {
        if self.kind != AgentKind::OpenCode {
            return Ok(RefreshResult { events: Vec::new() });
        }
        let full_context = link.parser_version.contains("full-export");
        let events = self
            .opencode_export(&link.external_session_id, full_context)
            .await?;
        for event in &events {
            sink.push(event.clone()).await?;
        }
        Ok(RefreshResult { events })
    }
}

fn write_claude_handoff(bootstrap: &str) -> Result<NamedTempFile, AdapterError> {
    let mut file = NamedTempFile::new().map_err(|error| {
        AdapterError::Other(format!("could not create Claude handoff file: {error}"))
    })?;
    file.write_all(bootstrap.as_bytes()).map_err(|error| {
        AdapterError::Other(format!("could not write Claude handoff file: {error}"))
    })?;
    file.flush().map_err(|error| {
        AdapterError::Other(format!("could not flush Claude handoff file: {error}"))
    })?;
    Ok(file)
}

fn profile(kind: AgentKind, version: Option<&str>) -> String {
    let known = version
        .and_then(version_pair)
        .is_some_and(|pair| match kind {
            AgentKind::OpenCode => pair == (1, 18),
            AgentKind::ClaudeCode => pair == (2, 1),
            AgentKind::Codex => pair == (0, 145),
        });
    if !known {
        return "unverified-launch-only".to_owned();
    }
    match kind {
        AgentKind::OpenCode => OPENCODE_PROFILE,
        AgentKind::ClaudeCode => CLAUDE_PROFILE,
        AgentKind::Codex => CODEX_PROFILE,
    }
    .to_owned()
}

fn capabilities(kind: AgentKind, profile: &str) -> AgentCapabilities {
    match (kind, profile) {
        (AgentKind::OpenCode, OPENCODE_PROFILE) => AgentCapabilities {
            session_discovery: true,
            session_import: true,
            native_resume: true,
            initial_prompt_argument: true,
            structured_export: true,
            server_api: true,
            interactive_pty: true,
            ..AgentCapabilities::default()
        },
        (AgentKind::ClaudeCode, CLAUDE_PROFILE) => AgentCapabilities {
            native_resume: true,
            initial_prompt_argument: true,
            lifecycle_hooks: true,
            interactive_pty: true,
            ..AgentCapabilities::default()
        },
        (AgentKind::Codex, CODEX_PROFILE) => AgentCapabilities {
            native_resume: true,
            initial_prompt_argument: true,
            interactive_pty: true,
            ..AgentCapabilities::default()
        },
        _ => AgentCapabilities {
            interactive_pty: true,
            ..AgentCapabilities::default()
        },
    }
}

fn opencode_export_args(id: &ExternalSessionId, full_context: bool) -> Vec<&str> {
    let mut args = vec!["export", id.as_str()];
    if !full_context {
        args.push("--sanitize");
    }
    args
}

fn version_pair(value: &str) -> Option<(u64, u64)> {
    value
        .split(|character: char| !(character.is_ascii_digit() || character == '.'))
        .find_map(|candidate| {
            let mut pieces = candidate.split('.');
            Some((pieces.next()?.parse().ok()?, pieces.next()?.parse().ok()?))
        })
}

fn parse_opencode_sessions(
    bytes: &[u8],
    fallback_project: Option<&Path>,
) -> Result<Vec<DiscoveredSession>, AdapterError> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|error| malformed_list(error.to_string()))?;
    let rows = value
        .as_array()
        .or_else(|| value.pointer("/data/sessions").and_then(Value::as_array))
        .ok_or_else(|| malformed_list("expected a JSON array of sessions".into()))?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            let id = field(row, &["id"]).or_else(|| field(row, &["sessionID"]))?;
            let external_session_id = ExternalSessionId::new(id.to_owned()).ok()?;
            Some(DiscoveredSession {
                agent: AgentKind::OpenCode,
                external_session_id,
                project_path: field(row, &["directory"])
                    .or_else(|| field(row, &["path"]))
                    .map(PathBuf::from)
                    .or_else(|| fallback_project.map(Path::to_path_buf)),
                updated_at: timestamp(row).unwrap_or(DateTime::<Utc>::UNIX_EPOCH),
                first_user_request: field(row, &["title"]).map(str::to_owned),
                approximate_event_count: row
                    .get("messageCount")
                    .and_then(Value::as_u64)
                    .unwrap_or_default() as usize,
                already_imported: false,
                source_path: None,
            })
        })
        .collect())
}

fn parse_opencode_export(
    bytes: &[u8],
    namespace: &str,
    parser_version: &str,
) -> Result<Vec<cb_core::NewContextEvent>, AdapterError> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|error| malformed_export(error.to_string()))?;
    let messages = value
        .get("messages")
        .or_else(|| value.pointer("/data/messages"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            malformed_export("documented export did not include a messages array".into())
        })?;
    let scanner = LocalSecretScanner::default();
    let context = NormalizationContext {
        agent: AgentKind::OpenCode,
        parser_name: "opencode-export",
        parser_version,
        external_session_namespace: namespace,
        source_path: None,
    };
    let events = messages
        .iter()
        .enumerate()
        .flat_map(|(ordinal, message)| {
            let info = message.get("info").unwrap_or(message);
            let mut raw_events = Vec::new();
            let kind = match field(info, &["role"]) {
                Some("user") => Some(ContextEventKind::UserMessage),
                Some("assistant") => Some(ContextEventKind::AssistantMessage),
                Some("system") => Some(ContextEventKind::SystemMessage),
                _ => None,
            };
            let content = message_content(message);
            if let Some(kind) = kind.filter(|_| !content.is_empty()) {
                raw_events.push(normalize_raw_event(
                    RawEvent {
                        external_event_id: field(info, &["id"]).map(str::to_owned),
                        timestamp: timestamp(info),
                        kind,
                        payload: ContextEventPayload::Message { content },
                        metadata: message.clone(),
                    },
                    context,
                    ordinal,
                    &scanner,
                ));
            }
            raw_events.extend(tool_events(message, info, ordinal, context, &scanner));
            raw_events
        })
        .collect::<Vec<_>>();
    if events.is_empty() {
        return Err(malformed_export("no supported messages were found".into()));
    }
    Ok(events)
}

fn field<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter()
        .try_fold(value, |value, key| value.get(*key))?
        .as_str()
}
fn timestamp(value: &Value) -> Option<DateTime<Utc>> {
    value
        .get("updatedAt")
        .or_else(|| value.get("updated"))
        .or_else(|| value.get("createdAt"))
        .or_else(|| value.get("created"))
        .or_else(|| value.pointer("/time/updated"))
        .or_else(|| value.pointer("/time/created"))
        .and_then(timestamp_value)
}

fn timestamp_value(value: &Value) -> Option<DateTime<Utc>> {
    if let Some(value) = value.as_str() {
        return DateTime::parse_from_rfc3339(value)
            .ok()
            .map(|value| value.with_timezone(&Utc));
    }
    let epoch = value.as_i64()?;
    DateTime::from_timestamp_millis(epoch).or_else(|| DateTime::from_timestamp(epoch, 0))
}

fn tool_events(
    message: &Value,
    info: &Value,
    ordinal: usize,
    context: NormalizationContext<'_>,
    scanner: &LocalSecretScanner,
) -> Vec<cb_core::NewContextEvent> {
    let message_id = field(info, &["id"]).unwrap_or("message");
    let timestamp = timestamp(info);
    message
        .get("parts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
        .filter(|(_, part)| {
            field(part, &["type"]).is_some_and(|part_type| part_type.contains("tool"))
                || part.get("tool").is_some()
                || part.get("toolName").is_some()
        })
        .flat_map(|(part_ordinal, part)| {
            let part_id = field(part, &["id"])
                .map(str::to_owned)
                .unwrap_or_else(|| format!("{message_id}-{part_ordinal}"));
            let call_id = field(part, &["callID"])
                .or_else(|| field(part, &["callId"]))
                .unwrap_or(&part_id)
                .to_owned();
            let name = field(part, &["tool"])
                .or_else(|| field(part, &["toolName"]))
                .or_else(|| field(part, &["name"]))
                .unwrap_or("opencode_tool")
                .to_owned();
            let input = part
                .pointer("/state/input")
                .or_else(|| part.get("input"))
                .cloned();
            let output = part
                .pointer("/state/output")
                .or_else(|| part.get("output"))
                .cloned();
            let mut events = Vec::new();
            if let Some(input) = input {
                events.push(normalize_raw_event(
                    RawEvent {
                        external_event_id: Some(format!("{part_id}:call")),
                        timestamp,
                        kind: ContextEventKind::ToolCall,
                        payload: ContextEventPayload::ToolCall { name, input },
                        metadata: part.clone(),
                    },
                    context,
                    ordinal
                        .saturating_mul(1000)
                        .saturating_add(part_ordinal * 2),
                    scanner,
                ));
            }
            if let Some(output) = output {
                let summary = serde_json::to_string(&output)
                    .unwrap_or_default()
                    .chars()
                    .take(4096)
                    .collect();
                events.push(normalize_raw_event(
                    RawEvent {
                        external_event_id: Some(format!("{part_id}:result")),
                        timestamp,
                        kind: ContextEventKind::ToolResult,
                        payload: ContextEventPayload::ToolResult {
                            tool_call_id: Some(call_id),
                            summary,
                            content_hash: None,
                            artifact_path: None,
                            success: !matches!(
                                field(part, &["state", "status"]),
                                Some("error" | "failed")
                            ),
                        },
                        metadata: part.clone(),
                    },
                    context,
                    ordinal
                        .saturating_mul(1000)
                        .saturating_add(part_ordinal * 2 + 1),
                    scanner,
                ));
            }
            events
        })
        .collect()
}
fn message_content(message: &Value) -> String {
    let mut content = field(message, &["content"])
        .map(str::to_owned)
        .into_iter()
        .collect::<Vec<_>>();
    if let Some(parts) = message.get("parts").and_then(Value::as_array) {
        content.extend(
            parts
                .iter()
                .filter(|part| !matches!(field(part, &["type"]), Some("reasoning" | "analysis")))
                .filter_map(|part| {
                    field(part, &["text"])
                        .or_else(|| field(part, &["content"]))
                        .map(str::to_owned)
                }),
        );
    }
    content.join("\n")
}
fn malformed_list(details: String) -> AdapterError {
    AdapterError::MalformedSession {
        path: PathBuf::from("<opencode session list>"),
        details,
    }
}
fn malformed_export(details: String) -> AdapterError {
    AdapterError::MalformedSession {
        path: PathBuf::from("<opencode export>"),
        details,
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_opencode_export, parse_opencode_sessions, profile, write_claude_handoff};
    use cb_core::{AgentKind, ContextEventKind};
    #[test]
    fn known_versions_enable_only_guarded_profiles() {
        assert_eq!(
            profile(AgentKind::OpenCode, Some("1.18.4")),
            "opencode-cli-1.18"
        );
        assert_eq!(
            profile(AgentKind::OpenCode, Some("1.19.0")),
            "unverified-launch-only"
        );
    }
    #[test]
    fn parses_documented_opencode_json() {
        assert_eq!(
            parse_opencode_sessions(
                br#"[{"id":"ses_1","title":"Fix auth","updatedAt":"2026-07-24T00:00:00Z"}]"#,
                None
            )
            .expect("sessions")
            .len(),
            1
        );
        assert_eq!(parse_opencode_export(br#"{"messages":[{"info":{"id":"m1","role":"user"},"parts":[{"text":"Fix auth"}]},{"info":{"id":"m2","role":"assistant"},"parts":[{"text":"Done"}]}]}"#, "ses_1", "test-v1").expect("events").len(), 2);
    }

    #[test]
    fn fixture_export_keeps_message_and_tool_provenance() {
        let sessions = parse_opencode_sessions(
            include_bytes!("../tests/fixtures/opencode/v1.18/sessions.json"),
            None,
        )
        .expect("fixture sessions");
        assert_eq!(sessions[0].external_session_id.as_str(), "ses_fixture_1");
        let events = parse_opencode_export(
            include_bytes!("../tests/fixtures/opencode/v1.18/export.json"),
            "ses_fixture_1",
            "test-v1",
        )
        .expect("fixture export");
        assert_eq!(events.len(), 4);
        assert!(
            events
                .iter()
                .any(|event| event.kind == ContextEventKind::ToolCall)
        );
        assert!(
            events
                .iter()
                .any(|event| event.kind == ContextEventKind::ToolResult)
        );
        assert!(events.iter().all(|event| event.import_metadata.is_some()));
    }

    #[test]
    fn parses_live_style_epoch_millisecond_session_timestamp() {
        let sessions = parse_opencode_sessions(
            br#"[{"id":"ses_live","updated":1784842375015,"created":1784842000000}]"#,
            None,
        )
        .expect("sessions");
        assert_ne!(sessions[0].updated_at, chrono::DateTime::UNIX_EPOCH);
    }

    #[test]
    fn claude_handoff_is_written_to_a_private_file() {
        let content = "latest handoff context\n".repeat(20_000);
        let handoff = write_claude_handoff(&content).expect("handoff file");
        assert_eq!(
            std::fs::read_to_string(handoff.path()).expect("handoff content"),
            content
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            assert_eq!(
                std::fs::metadata(handoff.path())
                    .expect("handoff metadata")
                    .permissions()
                    .mode()
                    & 0o077,
                0
            );
        }
    }
}
