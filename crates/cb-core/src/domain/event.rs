use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{AgentKind, BridgeSessionId, EventId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sensitivity {
    Normal,
    PotentialSecret,
    Secret,
    Excluded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextEventKind {
    UserMessage,
    AssistantMessage,
    SystemMessage,
    ToolCall,
    ToolResult,
    CommandExecuted,
    FileRead,
    FileCreated,
    FileModified,
    FileDeleted,
    FileMoved,
    GitState,
    GitDiff,
    TestRun,
    Decision,
    Assumption,
    Error,
    Checkpoint,
    Handoff,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ContextEventPayload {
    Message {
        content: String,
    },
    ToolCall {
        name: String,
        input: Value,
    },
    ToolResult {
        tool_call_id: Option<String>,
        summary: String,
        content_hash: Option<String>,
        artifact_path: Option<PathBuf>,
        success: bool,
    },
    Command {
        command: String,
        cwd: PathBuf,
        exit_code: Option<i32>,
        output_summary: Option<String>,
    },
    File {
        path: PathBuf,
        previous_path: Option<PathBuf>,
        summary: Option<String>,
    },
    GitState {
        branch: Option<String>,
        head: Option<String>,
        status: String,
        staged_diff: String,
        unstaged_diff: String,
        untracked_files: Vec<PathBuf>,
        #[serde(default)]
        filesystem_file_count: usize,
        #[serde(default)]
        filesystem_fingerprint: String,
    },
    GitDiff {
        diff: String,
        truncated: bool,
    },
    TestRun {
        command: String,
        outcome: TestOutcome,
        summary: String,
    },
    Decision {
        decision: String,
        rationale: Option<String>,
    },
    Assumption {
        assumption: String,
    },
    Error {
        message: String,
        context: Option<String>,
        resolved: bool,
    },
    Checkpoint {
        note: Option<String>,
        completed_work: Vec<String>,
        pending_tasks: Vec<String>,
        recommended_next_action: Option<String>,
    },
    Handoff {
        target_agent: AgentKind,
        handoff_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestOutcome {
    Passed,
    Failed,
    Skipped,
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportMetadata {
    pub parser_name: String,
    pub parser_version: String,
    pub source_path: Option<PathBuf>,
    pub imported_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextEvent {
    pub id: EventId,
    pub bridge_session_id: BridgeSessionId,
    pub source_agent: Option<AgentKind>,
    pub external_event_id: Option<String>,
    pub sequence: i64,
    pub timestamp: DateTime<Utc>,
    pub kind: ContextEventKind,
    pub payload: ContextEventPayload,
    pub content_hash: String,
    pub sensitivity: Sensitivity,
    pub import_metadata: Option<ImportMetadata>,
    pub parent_event_id: Option<EventId>,
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewContextEvent {
    pub source_agent: Option<AgentKind>,
    pub external_event_id: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub kind: ContextEventKind,
    pub payload: ContextEventPayload,
    pub sensitivity: Sensitivity,
    pub import_metadata: Option<ImportMetadata>,
    pub parent_event_id: Option<EventId>,
    #[serde(default)]
    pub metadata: Value,
    pub import_key: String,
}

impl NewContextEvent {
    pub fn content_hash(&self) -> Result<String, serde_json::Error> {
        serde_json::to_vec(&self.payload).map(|bytes| blake3::hash(&bytes).to_hex().to_string())
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use serde_json::Value;

    use super::{ContextEventKind, ContextEventPayload, NewContextEvent, Sensitivity};

    #[test]
    fn content_hash_is_stable_and_payload_sensitive() {
        let first_event = event("first");
        assert_eq!(
            first_event.content_hash().expect("hash"),
            first_event.content_hash().expect("hash")
        );
        assert_ne!(
            first_event.content_hash().expect("hash"),
            event("second").content_hash().expect("hash")
        );
    }

    #[test]
    fn legacy_git_state_payloads_deserialize_with_snapshot_defaults() {
        let payload: ContextEventPayload = serde_json::from_str(
            r#"{
                "type": "git_state",
                "data": {
                    "branch": "main",
                    "head": "abc123",
                    "status": "",
                    "staged_diff": "",
                    "unstaged_diff": "",
                    "untracked_files": []
                }
            }"#,
        )
        .expect("legacy Git state payload");
        assert!(matches!(
            payload,
            ContextEventPayload::GitState {
                filesystem_file_count: 0,
                filesystem_fingerprint,
                ..
            } if filesystem_fingerprint.is_empty()
        ));
    }

    fn event(content: &str) -> NewContextEvent {
        NewContextEvent {
            source_agent: None,
            external_event_id: None,
            timestamp: Utc::now(),
            kind: ContextEventKind::UserMessage,
            payload: ContextEventPayload::Message {
                content: content.to_owned(),
            },
            sensitivity: Sensitivity::Normal,
            import_metadata: None,
            parent_event_id: None,
            metadata: Value::Null,
            import_key: content.to_owned(),
        }
    }
}
