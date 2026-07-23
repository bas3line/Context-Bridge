use std::fmt::Write;

use cb_core::{BridgeSessionId, EventRepository, HandoffPackage, SecretScanner, Sensitivity};
use miette::{IntoDiagnostic, WrapErr};

use crate::{
    commands::{App, ExportFormat},
    output::{print_json, terminal_safe},
};

pub async fn execute(
    app: &App,
    session_id: BridgeSessionId,
    format: ExportFormat,
    redacted: bool,
) -> miette::Result<i32> {
    let session = app.require_current_project_session(session_id).await?;
    let events = app.store.events(session_id).await.into_diagnostic()?;
    let handoff = app
        .store
        .latest_handoff(session_id)
        .await
        .into_diagnostic()?;
    match format {
        ExportFormat::Json if redacted => {
            let scanner = app.scanner();
            let safe_handoff = redacted_handoff_value(handoff.as_ref(), &scanner)
                .into_diagnostic()
                .wrap_err("could not redact the derived handoff package")?;
            let safe_events = events
                .iter()
                .filter(|event| event.sensitivity != Sensitivity::Excluded)
                .map(|event| {
                    let payload =
                        if event.sensitivity == Sensitivity::Secret {
                            "[REDACTED SECRET EVENT]".to_owned()
                        } else {
                            scanner.redact(&serde_json::to_string(&event.payload).unwrap_or_else(
                                |_| "[unserializable canonical payload]".to_owned(),
                            ))
                        };
                    serde_json::json!({
                        "id": event.id,
                        "sequence": event.sequence,
                        "timestamp": event.timestamp,
                        "source_agent": event.source_agent,
                        "kind": event.kind,
                        "sensitivity": event.sensitivity,
                        "payload": payload,
                        "content_hash": event.content_hash,
                    })
                })
                .collect::<Vec<_>>();
            print_json(&serde_json::json!({
                "schema_version": 1,
                "session": session,
                "events": safe_events,
                "latest_handoff": safe_handoff,
                "redacted": true,
            }))?;
        }
        ExportFormat::Json => {
            print_json(&serde_json::json!({
                "schema_version": 1,
                "session": session,
                "events": events,
                "latest_handoff": handoff,
                "redacted": false,
            }))?;
        }
        ExportFormat::Markdown => {
            let scanner = app.scanner();
            let mut output = format!(
                "# Context Bridge session {}\n\n- Status: {:?}\n- Active agent: {}\n- Updated: {}\n",
                session.id,
                session.status,
                session
                    .active_agent
                    .map_or_else(|| "none".to_owned(), |agent| agent.to_string()),
                session.updated_at
            );
            output.push_str("\n## Timeline\n");
            for event in &events {
                if event.sensitivity == Sensitivity::Excluded {
                    continue;
                }
                let mut summary = super::timeline::summarize(event);
                if redacted {
                    summary = if event.sensitivity == Sensitivity::Secret {
                        "[REDACTED SECRET EVENT]".to_owned()
                    } else {
                        scanner.redact(&summary)
                    };
                }
                let _ = writeln!(
                    output,
                    "\n### {} · {:?} · {}\n\n{}",
                    event.sequence, event.kind, event.timestamp, summary
                );
            }
            let handoff = if redacted {
                redacted_handoff_value(handoff.as_ref(), &scanner)
                    .into_diagnostic()
                    .wrap_err("could not redact the derived handoff package")?
            } else {
                handoff
                    .as_ref()
                    .map(serde_json::to_value)
                    .transpose()
                    .into_diagnostic()
                    .wrap_err("could not serialize the derived handoff package")?
            };
            if let Some(handoff) = handoff {
                output.push_str("\n## Latest derived handoff\n\n```json\n");
                let serialized = serde_json::to_string_pretty(&handoff).into_diagnostic()?;
                output.push_str(&serialized);
                output.push_str("\n```\n");
            }
            print!("{}", terminal_safe(&output));
        }
    }
    Ok(0)
}

fn redacted_handoff_value(
    handoff: Option<&HandoffPackage>,
    scanner: &dyn SecretScanner,
) -> Result<Option<serde_json::Value>, serde_json::Error> {
    handoff
        .map(|package| {
            let serialized = serde_json::to_string(package)?;
            if scanner.classify(&serialized) == Sensitivity::Normal {
                serde_json::to_value(package)
            } else {
                Ok(serde_json::json!({
                    "redacted": true,
                    "reason": "latest handoff omitted because it matched the active secret policy"
                }))
            }
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use cb_core::{AgentKind, BridgeSessionId, GitContext, HandoffId, ProjectSummary};
    use cb_security::LocalSecretScanner;
    use chrono::Utc;

    use super::{HandoffPackage, redacted_handoff_value};

    #[test]
    fn redacted_export_omits_an_entire_handoff_that_contains_a_secret() {
        let package = HandoffPackage {
            id: HandoffId::new(),
            schema_version: 1,
            session_id: BridgeSessionId::new(),
            source_agent: AgentKind::ClaudeCode,
            target_agent: AgentKind::Codex,
            project: ProjectSummary {
                id: "project".to_owned(),
                root: PathBuf::from("/project"),
            },
            original_objective: None,
            current_objective: Some("API_KEY=top-secret-value".to_owned()),
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
            git: GitContext::default(),
            generated_at: Utc::now(),
        };

        let output = redacted_handoff_value(Some(&package), &LocalSecretScanner::default())
            .expect("package serializes")
            .expect("package is represented");
        let serialized = serde_json::to_string(&output).expect("output serializes");

        assert!(!serialized.contains("top-secret-value"));
        assert_eq!(output["redacted"], true);
    }
}
