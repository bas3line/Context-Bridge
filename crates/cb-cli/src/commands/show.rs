use cb_core::{BridgeSessionId, EventRepository, SessionRepository};
use miette::{IntoDiagnostic, miette};

use crate::{
    commands::App,
    output::{print_field, print_json, print_table},
};

pub async fn execute(app: &App, session_id: BridgeSessionId) -> miette::Result<i32> {
    let session = app.require_current_project_session(session_id).await?;
    let project = app
        .store
        .project(&session.project_id)
        .await
        .into_diagnostic()?
        .ok_or_else(|| miette!("project `{}` was not found", session.project_id))?;
    let links = app
        .store
        .external_links(session_id)
        .await
        .into_diagnostic()?;
    let checkpoints = app.store.checkpoints(session_id).await.into_diagnostic()?;
    let event_count = app.store.event_count(session_id).await.into_diagnostic()?;
    if app.json {
        print_json(&serde_json::json!({
            "session": session,
            "project": project,
            "external_links": links,
            "checkpoints": checkpoints,
            "event_count": event_count,
        }))?;
    } else {
        print_field("Session", session.id);
        print_field("Project", project.root.display());
        print_field("Status", format!("{:?}", session.status).to_lowercase());
        print_field(
            "Active agent",
            session
                .active_agent
                .map_or_else(|| "-".to_owned(), |agent| agent.to_string()),
        );
        print_field("Events", event_count);
        let rows = links
            .iter()
            .map(|link| {
                vec![
                    link.agent.to_string(),
                    link.external_session_id.to_string(),
                    link.last_synced_at
                        .map_or_else(|| "-".to_owned(), |value| value.to_rfc3339()),
                    link.parser_version.clone(),
                ]
            })
            .collect::<Vec<_>>();
        print_table(&["AGENT", "EXTERNAL ID", "LAST SYNC", "PARSER"], &rows);
    }
    Ok(0)
}
