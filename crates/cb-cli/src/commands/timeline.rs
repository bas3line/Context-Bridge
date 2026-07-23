use cb_core::{
    BridgeSessionId, ContextEvent, ContextEventPayload, EventRepository, SecretScanner, Sensitivity,
};
use miette::IntoDiagnostic;

use crate::{
    commands::App,
    output::{print_json, print_table},
};

pub async fn execute(
    app: &App,
    session_id: BridgeSessionId,
    include_sensitive: bool,
) -> miette::Result<i32> {
    app.require_current_project_session(session_id).await?;
    let events = app.store.events(session_id).await.into_diagnostic()?;
    let events = if include_sensitive {
        events
    } else {
        events
            .into_iter()
            .filter(|event| event.sensitivity == Sensitivity::Normal)
            .collect()
    };
    if app.json {
        print_json(&events)?;
    } else {
        let scanner = app.scanner();
        let rows = events
            .iter()
            .map(|event| {
                vec![
                    event.sequence.to_string(),
                    event.timestamp.to_rfc3339(),
                    event
                        .source_agent
                        .map_or_else(|| "bridge".to_owned(), |agent| agent.to_string()),
                    format!("{:?}", event.kind).to_lowercase(),
                    scanner.redact(&summarize(event)),
                ]
            })
            .collect::<Vec<_>>();
        print_table(&["SEQ", "TIME", "SOURCE", "KIND", "SUMMARY"], &rows);
    }
    Ok(0)
}

pub(crate) fn summarize(event: &ContextEvent) -> String {
    let summary = match &event.payload {
        ContextEventPayload::Message { content } => content.clone(),
        ContextEventPayload::ToolCall { name, .. } => format!("tool call: {name}"),
        ContextEventPayload::ToolResult { summary, .. } => summary.clone(),
        ContextEventPayload::Command { command, .. } => command.clone(),
        ContextEventPayload::File { path, .. } => path.display().to_string(),
        ContextEventPayload::GitState { status, .. } => status.clone(),
        ContextEventPayload::GitDiff { diff, .. } => diff.clone(),
        ContextEventPayload::TestRun { summary, .. } => summary.clone(),
        ContextEventPayload::Decision { decision, .. } => decision.clone(),
        ContextEventPayload::Assumption { assumption } => assumption.clone(),
        ContextEventPayload::Error { message, .. } => message.clone(),
        ContextEventPayload::Checkpoint { note, .. } => {
            note.clone().unwrap_or_else(|| "checkpoint".to_owned())
        }
        ContextEventPayload::Handoff { target_agent, .. } => {
            format!("handoff to {target_agent}")
        }
    };
    summary.replace('\n', " ").chars().take(120).collect()
}
