use cb_core::{BridgeSessionId, ContextEventPayload, EventRepository, SecretScanner};
use miette::{IntoDiagnostic, miette};

use crate::{
    commands::App,
    output::{print_json, terminal_safe},
};

pub async fn execute(app: &App, session_id: BridgeSessionId) -> miette::Result<i32> {
    app.require_current_project_session(session_id).await?;
    let events = app.store.events(session_id).await.into_diagnostic()?;
    let diff = events.iter().rev().find_map(|event| match &event.payload {
        ContextEventPayload::GitDiff { diff, truncated } => Some((diff, truncated)),
        _ => None,
    });
    let Some((diff, truncated)) = diff else {
        return Err(miette!(
            "session `{session_id}` has no captured Git diff; it may not be a Git project"
        ));
    };
    let safe = app.scanner().redact(diff);
    if app.json {
        print_json(&serde_json::json!({
            "session_id": session_id,
            "diff": safe,
            "truncated": truncated,
        }))?;
    } else {
        print!("{}", terminal_safe(&safe));
    }
    Ok(0)
}
