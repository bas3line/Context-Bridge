use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{AgentKind, BridgeSessionId, CanonicalMessage, HandoffId, TestOutcome};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSummary {
    pub id: String,
    pub root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItem {
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateItem {
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionRecord {
    pub decision: String,
    pub rationale: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssumptionRecord {
    pub assumption: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailedApproach {
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    pub path: PathBuf,
    pub change: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelevantFile {
    pub path: PathBuf,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandRecord {
    pub command: String,
    pub exit_code: Option<i32>,
    pub output_summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestRecord {
    pub command: String,
    pub outcome: TestOutcome,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorRecord {
    pub message: String,
    pub resolved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskItem {
    pub task: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GitContext {
    pub branch: Option<String>,
    pub head: Option<String>,
    pub status: String,
    pub staged_diff: String,
    pub unstaged_diff: String,
    pub untracked_files: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HandoffPackage {
    pub id: HandoffId,
    pub schema_version: u32,
    pub session_id: BridgeSessionId,
    pub source_agent: AgentKind,
    pub target_agent: AgentKind,
    pub project: ProjectSummary,
    pub original_objective: Option<String>,
    pub current_objective: Option<String>,
    pub completed_work: Vec<WorkItem>,
    pub current_state: Vec<StateItem>,
    pub decisions: Vec<DecisionRecord>,
    pub assumptions: Vec<AssumptionRecord>,
    pub failed_approaches: Vec<FailedApproach>,
    pub modified_files: Vec<FileChange>,
    pub relevant_files: Vec<RelevantFile>,
    pub commands: Vec<CommandRecord>,
    pub tests: Vec<TestRecord>,
    pub errors: Vec<ErrorRecord>,
    pub pending_tasks: Vec<TaskItem>,
    pub recommended_next_action: Option<String>,
    pub recent_conversation: Vec<CanonicalMessage>,
    pub git: GitContext,
    pub generated_at: DateTime<Utc>,
}
