use cb_core::HandoffPackage;

use crate::{ContextError, TokenEstimator};

pub fn compact_to_budget(
    package: &mut HandoffPackage,
    budget: usize,
    estimator: &dyn TokenEstimator,
) -> Result<(), ContextError> {
    if budget == 0 {
        return Err(ContextError::InvalidBudget);
    }
    macro_rules! discard_oldest {
        ($field:ident) => {
            while estimated(package, estimator)? > budget && !package.$field.is_empty() {
                package.$field.remove(0);
            }
        };
    }

    // Remove lowest-priority, oldest material first. Objectives are stored
    // separately and never discarded here; the remaining phases are ordered
    // from contextual detail toward the facts the target needs to act safely.
    discard_oldest!(failed_approaches);
    discard_oldest!(commands);
    discard_oldest!(relevant_files);
    discard_oldest!(recent_conversation);
    discard_oldest!(completed_work);
    discard_oldest!(tests);
    discard_oldest!(pending_tasks);

    // Diffs are high-value but can be disproportionately large. Preserve a
    // bounded prefix rather than dropping the whole Git context before facts
    // such as errors, decisions, and changed-file names are considered.
    if estimated(package, estimator)? > budget {
        let per_diff_budget = budget / 16;
        truncate_to_tokens(&mut package.git.staged_diff, per_diff_budget);
        truncate_to_tokens(&mut package.git.unstaged_diff, per_diff_budget);
    }

    discard_oldest!(errors);
    discard_oldest!(decisions);
    discard_oldest!(assumptions);
    discard_oldest!(current_state);
    discard_oldest!(modified_files);
    let required = estimated(package, estimator)?;
    if required > budget {
        return Err(ContextError::BudgetTooSmall { budget, required });
    }
    Ok(())
}

fn estimated(
    package: &HandoffPackage,
    estimator: &dyn TokenEstimator,
) -> Result<usize, ContextError> {
    Ok(estimator.estimate(&serde_json::to_string(package)?))
}

fn truncate_to_tokens(value: &mut String, tokens: usize) {
    const MARKER: &str = "\n[truncated to context budget]\n";
    let byte_limit = tokens.saturating_mul(4);
    if value.len() <= byte_limit {
        return;
    }
    if byte_limit <= MARKER.len() {
        value.truncate(floor_char_boundary(value, byte_limit));
        return;
    }
    value.truncate(floor_char_boundary(value, byte_limit - MARKER.len()));
    value.push_str(MARKER);
}

fn floor_char_boundary(value: &str, index: usize) -> usize {
    let mut boundary = index.min(value.len());
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    boundary
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use cb_core::{
        AgentKind, BridgeSessionId, CommandRecord, DecisionRecord, ErrorRecord, GitContext,
        HandoffId, HandoffPackage, ProjectSummary, RelevantFile, StateItem, TaskItem,
    };
    use chrono::{DateTime, Utc};

    use crate::{ApproximateTokenEstimator, TokenEstimator, compact_to_budget};

    #[test]
    fn compaction_discards_low_priority_detail_before_decisions_errors_and_tasks() {
        let mut package = package();
        package.commands = (0..20)
            .map(|index| CommandRecord {
                command: format!("tool-{index}"),
                exit_code: Some(0),
                output_summary: Some("x".repeat(400)),
            })
            .collect();
        package.relevant_files = (0..10)
            .map(|index| RelevantFile {
                path: PathBuf::from(format!("src/verbose-{index}.rs")),
                reason: "y".repeat(200),
            })
            .collect();
        package.decisions.push(DecisionRecord {
            decision: "Keep the key rotation transaction atomic.".to_owned(),
            rationale: Some(
                "The user constraint requires no partially rotated credentials.".to_owned(),
            ),
        });
        package.errors.push(ErrorRecord {
            message: "A migration failed on the staging schema.".to_owned(),
            resolved: false,
        });
        package.pending_tasks.push(TaskItem {
            task: "Retry the migration after repairing the schema.".to_owned(),
        });

        let estimator = ApproximateTokenEstimator;
        compact_to_budget(&mut package, 700, &estimator).expect("compacts into budget");

        assert!(package.commands.is_empty());
        assert!(package.relevant_files.len() < 10);
        assert_eq!(package.decisions.len(), 1);
        assert_eq!(package.errors.len(), 1);
        assert_eq!(package.pending_tasks.len(), 1);
        assert!(estimator.estimate(&serde_json::to_string(&package).expect("package JSON")) <= 700);
    }

    fn package() -> HandoffPackage {
        HandoffPackage {
            id: HandoffId::new(),
            schema_version: 1,
            session_id: BridgeSessionId::new(),
            source_agent: AgentKind::ClaudeCode,
            target_agent: AgentKind::OpenCode,
            project: ProjectSummary {
                id: "project".to_owned(),
                root: "/tmp/project".into(),
            },
            original_objective: Some("Safely rotate credentials.".to_owned()),
            current_objective: Some("Safely rotate credentials.".to_owned()),
            completed_work: Vec::new(),
            current_state: vec![StateItem {
                summary: "Work is in progress.".to_owned(),
            }],
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
            git: GitContext::default(),
            generated_at: DateTime::<Utc>::UNIX_EPOCH,
        }
    }
}
