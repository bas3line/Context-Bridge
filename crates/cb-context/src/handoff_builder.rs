use std::collections::HashSet;

use cb_core::{
    AgentKind, AssumptionRecord, BridgeSessionId, CanonicalMessage, CommandRecord, ContextEvent,
    ContextEventKind, ContextEventPayload, DecisionRecord, ErrorRecord, FailedApproach, FileChange,
    GitContext, HandoffId, HandoffPackage, MessageRole, ProjectSummary, RelevantFile,
    SecretScanner, Sensitivity, StateItem, TaskItem, TestRecord, WorkItem,
};
use cb_security::PathPolicy;
use chrono::{DateTime, Utc};

use crate::{ApproximateTokenEstimator, ContextError, TokenEstimator, compact_to_budget};

pub struct HandoffRequest<'a> {
    pub session_id: BridgeSessionId,
    pub source_agent: AgentKind,
    pub target_agent: AgentKind,
    pub project: ProjectSummary,
    pub events: &'a [ContextEvent],
    pub git: GitContext,
    pub budget: usize,
}

pub struct HandoffBuilder<'a> {
    scanner: &'a dyn SecretScanner,
    path_policy: &'a PathPolicy,
    estimator: &'a dyn TokenEstimator,
}

impl<'a> HandoffBuilder<'a> {
    #[must_use]
    pub fn new(scanner: &'a dyn SecretScanner, path_policy: &'a PathPolicy) -> Self {
        static ESTIMATOR: ApproximateTokenEstimator = ApproximateTokenEstimator;
        Self {
            scanner,
            path_policy,
            estimator: &ESTIMATOR,
        }
    }

    #[must_use]
    pub fn with_estimator(mut self, estimator: &'a dyn TokenEstimator) -> Self {
        self.estimator = estimator;
        self
    }

    pub fn build(&self, request: HandoffRequest<'_>) -> Result<HandoffPackage, ContextError> {
        // An explicit raw OpenCode re-import can coexist with a prior sanitized
        // import of the same external event. Prefer the raw record for a local
        // handoff rather than showing an obsolete vendor placeholder.
        let full_export_event_ids = request
            .events
            .iter()
            .filter(|event| {
                event
                    .import_metadata
                    .as_ref()
                    .is_some_and(|metadata| metadata.parser_version == "opencode-export-full-v2")
            })
            .filter_map(|event| event.external_event_id.clone())
            .collect::<HashSet<_>>();
        let mut visible_events: Vec<_> = request
            .events
            .iter()
            .filter(|event| {
                !matches!(
                    event.sensitivity,
                    Sensitivity::Secret | Sensitivity::Excluded
                )
            })
            .filter(|event| {
                !event.import_metadata.as_ref().is_some_and(|metadata| {
                    metadata.parser_version.starts_with("opencode-export")
                        && metadata.parser_version != "opencode-export-full-v2"
                        && event
                            .external_event_id
                            .as_ref()
                            .is_some_and(|id| full_export_event_ids.contains(id))
                })
            })
            .collect();
        visible_events.sort_by(|left, right| {
            left.sequence
                .cmp(&right.sequence)
                .then_with(|| left.id.to_string().cmp(&right.id.to_string()))
        });
        let generated_at = handoff_watermark(&visible_events);
        let user_messages: Vec<_> = visible_events
            .iter()
            .filter(|event| event.kind == ContextEventKind::UserMessage)
            .filter_map(|event| message_content(event))
            .collect();

        let mut package = HandoffPackage {
            id: HandoffId::from_deterministic_seed(
                generated_at,
                b"context-bridge/handoff-id-placeholder/v1",
            ),
            schema_version: 1,
            session_id: request.session_id,
            source_agent: request.source_agent,
            target_agent: request.target_agent,
            project: request.project,
            original_objective: user_messages.first().map(|value| self.safe(value)),
            current_objective: user_messages.last().map(|value| self.safe(value)),
            completed_work: Vec::new(),
            current_state: Vec::new(),
            decisions: Vec::new(),
            assumptions: Vec::new(),
            failed_approaches: Vec::new(),
            modified_files: Vec::new(),
            relevant_files: Vec::new(),
            commands: Vec::new(),
            tests: Vec::new(),
            errors: Vec::new(),
            pending_tasks: Vec::new(),
            recommended_next_action: None,
            recent_conversation: Vec::new(),
            git: sanitize_git(request.git, self.scanner, self.path_policy),
            generated_at,
        };

        for event in visible_events {
            self.reduce_event(event, &mut package);
        }
        // Keep a recency-first window sized by the requested budget. A fixed
        // message count makes a 100k-token handoff behave like a small
        // transcript tail, while retaining the entire export would make the
        // compactor repeatedly serialize and discard thousands of messages.
        // Reserve the already-reduced structured state, then fill the rest of
        // the budget from the newest conversation backwards.
        let conversation = std::mem::take(&mut package.recent_conversation);
        let structured_size = self.estimator.estimate(&serde_json::to_string(&package)?);
        package.recent_conversation = latest_conversation_within_budget(
            conversation,
            request
                .budget
                .saturating_sub(structured_size.saturating_add(64)),
            self.estimator,
        )?;
        if package.current_state.is_empty() {
            package.current_state.push(StateItem {
                summary: if package.git.status.trim().is_empty() {
                    "Working tree is clean.".to_owned()
                } else {
                    "Working tree has captured changes; inspect the Git context below.".to_owned()
                },
            });
        }
        compact_to_budget(&mut package, request.budget, self.estimator)?;
        package.id = deterministic_handoff_id(&package)?;
        Ok(package)
    }

    fn reduce_event(&self, event: &ContextEvent, package: &mut HandoffPackage) {
        match &event.payload {
            ContextEventPayload::Message { content } => {
                let role = match event.kind {
                    ContextEventKind::UserMessage => Some(MessageRole::User),
                    ContextEventKind::AssistantMessage => Some(MessageRole::Assistant),
                    ContextEventKind::SystemMessage => Some(MessageRole::System),
                    _ => None,
                };
                if let Some(role) = role {
                    package.recent_conversation.push(CanonicalMessage {
                        role,
                        content: self.safe(content),
                        timestamp: event.timestamp,
                    });
                }
            }
            ContextEventPayload::Command {
                command,
                exit_code,
                output_summary,
                ..
            } => package.commands.push(CommandRecord {
                command: self.safe(command),
                exit_code: *exit_code,
                output_summary: output_summary.as_ref().map(|value| self.safe(value)),
            }),
            ContextEventPayload::File { path, summary, .. } => {
                if self.path_policy.is_excluded(path) {
                    return;
                }
                if event.kind == ContextEventKind::FileRead {
                    package.relevant_files.push(RelevantFile {
                        path: path.clone(),
                        reason: summary.as_ref().map_or_else(
                            || "Inspected during the session".to_owned(),
                            |value| self.safe(value),
                        ),
                    });
                } else {
                    package.modified_files.push(FileChange {
                        path: path.clone(),
                        change: event_kind_label(event.kind).to_owned(),
                    });
                }
            }
            ContextEventPayload::TestRun {
                command,
                outcome,
                summary,
            } => package.tests.push(TestRecord {
                command: self.safe(command),
                outcome: *outcome,
                summary: self.safe(summary),
            }),
            ContextEventPayload::Decision {
                decision,
                rationale,
            } => package.decisions.push(DecisionRecord {
                decision: self.safe(decision),
                rationale: rationale.as_ref().map(|value| self.safe(value)),
            }),
            ContextEventPayload::Assumption { assumption } => {
                package.assumptions.push(AssumptionRecord {
                    assumption: self.safe(assumption),
                });
            }
            ContextEventPayload::Error {
                message,
                context,
                resolved,
            } => {
                package.errors.push(ErrorRecord {
                    message: self.safe(message),
                    resolved: *resolved,
                });
                if *resolved {
                    package.failed_approaches.push(FailedApproach {
                        summary: context
                            .as_ref()
                            .map_or_else(|| self.safe(message), |value| self.safe(value)),
                    });
                }
            }
            ContextEventPayload::Checkpoint {
                note,
                completed_work,
                pending_tasks,
                recommended_next_action,
            } => {
                if let Some(note) = note {
                    package.current_state.push(StateItem {
                        summary: self.safe(note),
                    });
                }
                package
                    .completed_work
                    .extend(completed_work.iter().map(|summary| WorkItem {
                        summary: self.safe(summary),
                    }));
                package
                    .pending_tasks
                    .extend(pending_tasks.iter().map(|task| TaskItem {
                        task: self.safe(task),
                    }));
                if let Some(next) = recommended_next_action {
                    package.recommended_next_action = Some(self.safe(next));
                }
            }
            ContextEventPayload::GitState {
                status,
                filesystem_file_count,
                filesystem_fingerprint,
                ..
            } => {
                let summary = if filesystem_fingerprint.is_empty() {
                    "Project snapshot was captured before filesystem fingerprints were introduced."
                        .to_owned()
                } else {
                    let fingerprint = filesystem_fingerprint
                        .get(..12)
                        .unwrap_or(filesystem_fingerprint);
                    format!(
                        "Project snapshot: {filesystem_file_count} files, fingerprint {}.{}",
                        fingerprint,
                        if status.trim().is_empty() {
                            ""
                        } else {
                            " Git changes are present"
                        }
                    )
                };
                package.current_state.push(StateItem { summary });
            }
            ContextEventPayload::ToolCall { .. }
            | ContextEventPayload::ToolResult { .. }
            | ContextEventPayload::GitDiff { .. }
            | ContextEventPayload::Handoff { .. } => {}
        }
    }

    fn safe(&self, value: &str) -> String {
        self.scanner.redact(value)
    }
}

fn latest_conversation_within_budget(
    messages: Vec<CanonicalMessage>,
    budget: usize,
    estimator: &dyn TokenEstimator,
) -> Result<Vec<CanonicalMessage>, ContextError> {
    let mut newest_first = Vec::new();
    let mut used: usize = 0;
    for message in messages.into_iter().rev() {
        let cost = estimator.estimate(&serde_json::to_string(&message)?);
        if used.saturating_add(cost) > budget {
            break;
        }
        used = used.saturating_add(cost);
        newest_first.push(message);
    }
    newest_first.reverse();
    Ok(newest_first)
}

fn handoff_watermark(events: &[&ContextEvent]) -> DateTime<Utc> {
    events
        .iter()
        .map(|event| event.timestamp)
        .max()
        .unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
}

fn deterministic_handoff_id(package: &HandoffPackage) -> Result<HandoffId, ContextError> {
    let mut canonical = package.clone();
    canonical.id = HandoffId::from_deterministic_seed(
        canonical.generated_at,
        b"context-bridge/handoff-id-placeholder/v1",
    );
    let seed = serde_json::to_vec(&canonical)?;
    Ok(HandoffId::from_deterministic_seed(
        canonical.generated_at,
        &seed,
    ))
}

fn message_content(event: &ContextEvent) -> Option<&str> {
    match &event.payload {
        ContextEventPayload::Message { content } => Some(content),
        _ => None,
    }
}

fn event_kind_label(kind: ContextEventKind) -> &'static str {
    match kind {
        ContextEventKind::FileCreated => "created",
        ContextEventKind::FileDeleted => "deleted",
        ContextEventKind::FileMoved => "moved",
        ContextEventKind::FileModified => "modified",
        _ => "observed",
    }
}

fn sanitize_git(
    mut git: GitContext,
    scanner: &dyn SecretScanner,
    path_policy: &PathPolicy,
) -> GitContext {
    git.status = scanner.redact(&filter_status(&git.status, path_policy));
    git.staged_diff = scanner.redact(&filter_diff(&git.staged_diff, path_policy));
    git.unstaged_diff = scanner.redact(&filter_diff(&git.unstaged_diff, path_policy));
    git.untracked_files
        .retain(|path| !path_policy.is_excluded(path));
    git
}

fn filter_status(status: &str, path_policy: &PathPolicy) -> String {
    status
        .lines()
        .filter(|line| {
            if line.starts_with("##") {
                return true;
            }
            status_paths(line).is_some_and(|paths| {
                paths
                    .iter()
                    .all(|path| !path_policy.is_excluded(std::path::Path::new(path)))
            })
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn filter_diff(diff: &str, path_policy: &PathPolicy) -> String {
    let mut output = Vec::new();
    // An unparseable header is excluded rather than risking a secret-path
    // rename leaking through the handoff's defence-in-depth filter.
    let mut include = false;
    for line in diff.lines() {
        if line.starts_with("diff --git ") {
            include = diff_header_paths(line).is_some_and(|(old_path, new_path)| {
                !path_policy.is_excluded(std::path::Path::new(&old_path))
                    && !path_policy.is_excluded(std::path::Path::new(&new_path))
            });
        }
        if include {
            output.push(line);
        }
    }
    output.join("\n")
}

fn diff_header_paths(line: &str) -> Option<(String, String)> {
    let remainder = line.strip_prefix("diff --git ")?;
    let (old_path, remainder) = parse_git_path_token(remainder)?;
    let remainder = remainder.strip_prefix(' ')?;
    let (new_path, remainder) = parse_git_path_token(remainder)?;
    if !remainder.is_empty() {
        return None;
    }
    Some((
        old_path.strip_prefix("a/")?.to_owned(),
        new_path.strip_prefix("b/")?.to_owned(),
    ))
}

fn status_paths(line: &str) -> Option<Vec<String>> {
    let paths = line.get(3..)?;
    match split_status_rename(paths) {
        Some((old_path, new_path)) => Some(vec![
            parse_single_git_path(old_path)?,
            parse_single_git_path(new_path)?,
        ]),
        None => Some(vec![parse_single_git_path(paths)?]),
    }
}

fn split_status_rename(paths: &str) -> Option<(&str, &str)> {
    let mut quoted = false;
    let mut escaped = false;
    for (index, character) in paths.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match character {
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            _ if !quoted && paths[index..].starts_with(" -> ") => {
                return Some((&paths[..index], &paths[index + " -> ".len()..]));
            }
            _ => {}
        }
    }
    None
}

fn parse_single_git_path(value: &str) -> Option<String> {
    let (path, remainder) = parse_git_path_token(value.trim())?;
    remainder.is_empty().then_some(path)
}

fn parse_git_path_token(value: &str) -> Option<(String, &str)> {
    if let Some(value) = value.strip_prefix('"') {
        return parse_quoted_git_path(value);
    }
    let end = value.find(char::is_whitespace).unwrap_or(value.len());
    (end > 0).then(|| (value[..end].to_owned(), &value[end..]))
}

fn parse_quoted_git_path(value: &str) -> Option<(String, &str)> {
    let mut output = String::new();
    let mut characters = value.char_indices();
    while let Some((index, character)) = characters.next() {
        match character {
            '"' => return Some((output, &value[index + character.len_utf8()..])),
            '\\' => {
                let (_, escaped) = characters.next()?;
                match escaped {
                    '\\' | '"' => output.push(escaped),
                    'n' => output.push('\n'),
                    'r' => output.push('\r'),
                    't' => output.push('\t'),
                    '0'..='7' => return None,
                    _ => return None,
                }
            }
            _ => output.push(character),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::{
        path::{Path, PathBuf},
        str::FromStr,
    };

    use cb_core::{
        AgentKind, BridgeSessionId, ContextEvent, ContextEventKind, ContextEventPayload, EventId,
        GitContext, ProjectSummary, Sensitivity, TestOutcome,
    };
    use cb_security::{LocalSecretScanner, PathPolicy};
    use chrono::{DateTime, Utc};
    use serde_json::Value;

    use crate::{
        ApproximateTokenEstimator, HandoffBuilder, HandoffRequest, TokenEstimator,
        renderers::render_opencode,
    };

    #[test]
    fn deterministic_handoff_is_redacted_budgeted_and_matches_golden() {
        let session_id = BridgeSessionId::new();
        let timestamp = DateTime::<Utc>::from_str("2026-01-02T03:04:05Z").expect("timestamp");
        let events = vec![
            event(
                session_id,
                1,
                timestamp,
                ContextEventKind::UserMessage,
                ContextEventPayload::Message {
                    content: "Implement token rotation.".to_owned(),
                },
                Sensitivity::Normal,
            ),
            event(
                session_id,
                2,
                timestamp,
                ContextEventKind::AssistantMessage,
                ContextEventPayload::Message {
                    content: "Implemented persistence.".to_owned(),
                },
                Sensitivity::Normal,
            ),
            event(
                session_id,
                3,
                timestamp,
                ContextEventKind::Decision,
                ContextEventPayload::Decision {
                    decision: "Hash refresh tokens.".to_owned(),
                    rationale: Some("Avoid reusable plaintext in storage.".to_owned()),
                },
                Sensitivity::Normal,
            ),
            event(
                session_id,
                4,
                timestamp,
                ContextEventKind::FileModified,
                ContextEventPayload::File {
                    path: "src/auth.rs".into(),
                    previous_path: None,
                    summary: None,
                },
                Sensitivity::Normal,
            ),
            event(
                session_id,
                5,
                timestamp,
                ContextEventKind::FileRead,
                ContextEventPayload::File {
                    path: ".env".into(),
                    previous_path: None,
                    summary: Some("must be excluded".to_owned()),
                },
                Sensitivity::Normal,
            ),
            event(
                session_id,
                6,
                timestamp,
                ContextEventKind::CommandExecuted,
                ContextEventPayload::Command {
                    command: "cargo test auth".to_owned(),
                    cwd: PathBuf::from("."),
                    exit_code: Some(0),
                    output_summary: Some("Tests passed.".to_owned()),
                },
                Sensitivity::Normal,
            ),
            event(
                session_id,
                7,
                timestamp,
                ContextEventKind::TestRun,
                ContextEventPayload::TestRun {
                    command: "cargo test auth".to_owned(),
                    outcome: TestOutcome::Passed,
                    summary: "12 tests passed.".to_owned(),
                },
                Sensitivity::Normal,
            ),
            event(
                session_id,
                8,
                timestamp,
                ContextEventKind::Checkpoint,
                ContextEventPayload::Checkpoint {
                    note: Some("Persistence works.".to_owned()),
                    completed_work: vec!["Added token persistence.".to_owned()],
                    pending_tasks: vec!["Add replay detection.".to_owned()],
                    recommended_next_action: Some("Implement the replay test.".to_owned()),
                },
                Sensitivity::Normal,
            ),
            event(
                session_id,
                9,
                timestamp,
                ContextEventKind::AssistantMessage,
                ContextEventPayload::Message {
                    content: "API_KEY=golden-secret".to_owned(),
                },
                Sensitivity::Secret,
            ),
        ];
        let scanner = LocalSecretScanner::default();
        let policy = PathPolicy::new(&[]).expect("path policy");
        let package = HandoffBuilder::new(&scanner, &policy)
            .build(HandoffRequest {
                session_id,
                source_agent: AgentKind::ClaudeCode,
                target_agent: AgentKind::OpenCode,
                project: ProjectSummary {
                    id: "project".to_owned(),
                    root: "/tmp/project".into(),
                },
                events: &events,
                git: GitContext {
                    branch: Some("main".to_owned()),
                    head: Some("abc123".to_owned()),
                    status: " M src/auth.rs".to_owned(),
                    staged_diff: "diff --git a/.env b/safe.txt\nrename from .env\nrename to safe.txt\n+API_KEY=hidden\n\
                                  diff --git a/src/auth.rs b/src/auth.rs\n+safe change\n"
                        .to_owned(),
                    untracked_files: vec![".env".into()],
                    ..GitContext::default()
                },
                budget: 2_000,
            })
            .expect("handoff");
        assert!(package.relevant_files.is_empty());
        assert!(
            !serde_json::to_string(&package)
                .expect("JSON")
                .contains("golden-secret")
        );
        assert!(
            !serde_json::to_string(&package)
                .expect("JSON")
                .contains("hidden")
        );
        assert!(
            !package
                .git
                .untracked_files
                .iter()
                .any(|path| path == Path::new(".env"))
        );
        assert!(
            ApproximateTokenEstimator.estimate(&serde_json::to_string(&package).expect("JSON"))
                <= 2_000
        );
        assert_eq!(
            render_opencode(&package),
            include_str!("../../../tests/fixtures/golden/opencode-handoff.md")
        );
    }

    #[test]
    fn same_canonical_input_produces_identical_package_json() {
        let session_id = BridgeSessionId::new();
        let timestamp = DateTime::<Utc>::from_str("2026-01-02T03:04:05Z").expect("timestamp");
        let events = vec![event(
            session_id,
            1,
            timestamp,
            ContextEventKind::AssistantMessage,
            ContextEventPayload::Message {
                content: "Use access token: should-not-leak".to_owned(),
            },
            Sensitivity::PotentialSecret,
        )];
        let scanner = LocalSecretScanner::default();
        let policy = PathPolicy::new(&[]).expect("path policy");
        let builder = HandoffBuilder::new(&scanner, &policy);
        let build = || {
            builder.build(HandoffRequest {
                session_id,
                source_agent: AgentKind::ClaudeCode,
                target_agent: AgentKind::OpenCode,
                project: ProjectSummary {
                    id: "project".to_owned(),
                    root: "/tmp/project".into(),
                },
                events: &events,
                git: GitContext::default(),
                budget: 2_000,
            })
        };

        let first = build().expect("first handoff");
        let second = build().expect("second handoff");
        assert_eq!(
            serde_json::to_vec(&first).expect("first JSON"),
            serde_json::to_vec(&second).expect("second JSON"),
        );
        let rendered = render_opencode(&first);
        assert!(!rendered.contains("should-not-leak"));
        assert!(rendered.contains("[REDACTED BY CONTEXT BRIDGE]"));
    }

    #[test]
    fn conversation_window_uses_the_newest_messages_up_to_the_requested_budget() {
        let session_id = BridgeSessionId::new();
        let timestamp = DateTime::<Utc>::from_str("2026-01-02T03:04:05Z").expect("timestamp");
        let events = (0..48)
            .map(|index| {
                event(
                    session_id,
                    index + 1,
                    timestamp,
                    ContextEventKind::AssistantMessage,
                    ContextEventPayload::Message {
                        content: format!("message-{index}: {}", "x".repeat(180)),
                    },
                    Sensitivity::Normal,
                )
            })
            .collect::<Vec<_>>();
        let scanner = LocalSecretScanner::default();
        let policy = PathPolicy::new(&[]).expect("path policy");
        let package = HandoffBuilder::new(&scanner, &policy)
            .build(HandoffRequest {
                session_id,
                source_agent: AgentKind::OpenCode,
                target_agent: AgentKind::ClaudeCode,
                project: ProjectSummary {
                    id: "project".to_owned(),
                    root: "/tmp/project".into(),
                },
                events: &events,
                git: GitContext::default(),
                budget: 2_000,
            })
            .expect("handoff");

        assert!(package.recent_conversation.len() > 16);
        assert!(package.recent_conversation.len() < events.len());
        assert!(
            package
                .recent_conversation
                .last()
                .expect("latest message")
                .content
                .starts_with("message-47:")
        );
        assert!(
            !package
                .recent_conversation
                .first()
                .expect("oldest retained message")
                .content
                .starts_with("message-0:")
        );
        assert!(
            ApproximateTokenEstimator.estimate(&serde_json::to_string(&package).expect("JSON"))
                <= 2_000
        );
    }

    fn event(
        session_id: BridgeSessionId,
        sequence: i64,
        timestamp: DateTime<Utc>,
        kind: ContextEventKind,
        payload: ContextEventPayload,
        sensitivity: Sensitivity,
    ) -> ContextEvent {
        ContextEvent {
            id: EventId::new(),
            bridge_session_id: session_id,
            source_agent: Some(AgentKind::ClaudeCode),
            external_event_id: Some(format!("event-{sequence}")),
            sequence,
            timestamp,
            kind,
            content_hash: format!("hash-{sequence}"),
            payload,
            sensitivity,
            import_metadata: None,
            parent_event_id: None,
            metadata: Value::Null,
        }
    }
}
