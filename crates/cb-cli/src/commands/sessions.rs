use cb_core::{EventRepository, SessionRepository};
use miette::IntoDiagnostic;

use crate::{
    commands::App,
    output::{print_json, print_table},
};

pub async fn execute(app: &App) -> miette::Result<i32> {
    let sessions = app
        .store
        .list_sessions(Some(&app.project_id))
        .await
        .into_diagnostic()?;
    let mut output = Vec::with_capacity(sessions.len());
    for session in sessions {
        output.push(serde_json::json!({
            "id": session.id,
            "title": session.title,
            "status": session.status,
            "active_agent": session.active_agent,
            "updated_at": session.updated_at,
            "event_count": app.store.event_count(session.id).await.into_diagnostic()?,
        }));
    }
    if app.json {
        print_json(&output)?;
    } else {
        let rows = output
            .iter()
            .map(|session| {
                vec![
                    session["id"].as_str().unwrap_or("-").to_owned(),
                    session["active_agent"].as_str().unwrap_or("-").to_owned(),
                    session["status"].as_str().unwrap_or("-").to_owned(),
                    session["event_count"].to_string(),
                    session["updated_at"].as_str().unwrap_or("-").to_owned(),
                    session["title"].as_str().unwrap_or("-").to_owned(),
                ]
            })
            .collect::<Vec<_>>();
        print_table(
            &["SESSION", "AGENT", "STATUS", "EVENTS", "UPDATED", "TITLE"],
            &rows,
        );
    }
    Ok(0)
}
