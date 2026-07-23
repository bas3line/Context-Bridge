use std::io::{BufRead, IsTerminal, Write};

use async_trait::async_trait;
use cb_core::{
    AdapterError, AgentKind, BridgeSession, BridgeSessionId, DiscoveredSession, EventSink,
    ExternalSessionId, ExternalSessionLink, ImportSessionRequest, NewContextEvent, ProjectContext,
    SessionRepository, SessionStatus,
};
use chrono::Utc;
use miette::{IntoDiagnostic, WrapErr, miette};

use crate::{
    commands::App,
    output::{print_field, print_json, print_table},
};

pub async fn execute(
    app: &App,
    agent: AgentKind,
    external_id: Option<String>,
    full_context: bool,
) -> miette::Result<i32> {
    let adapter = app.adapters.get(agent).into_diagnostic()?;
    let parser_version = adapter
        .detect()
        .await
        .into_diagnostic()?
        .compatibility_profile;
    let (external_session_id, source_path, title) = if let Some(external_id) = external_id {
        (
            ExternalSessionId::new(external_id).into_diagnostic()?,
            None,
            None,
        )
    } else {
        let mut discovered = adapter
            .discover_sessions(Some(&ProjectContext {
                root: app.project.root.clone(),
            }))
            .await
            .into_diagnostic()
            .wrap_err("could not discover external sessions")?;
        for session in &mut discovered {
            session.already_imported = app
                .store
                .adapter_link_exists(agent, session.external_session_id.as_str())
                .await
                .into_diagnostic()?;
        }
        let Some(selected) = select_session(app, agent, &discovered)? else {
            return Ok(0);
        };
        (
            selected.external_session_id,
            selected.source_path,
            selected.first_user_request,
        )
    };

    let existing = app
        .store
        .session_for_external(agent, external_session_id.as_str())
        .await
        .into_diagnostic()?;
    if let Some(existing_session_id) = existing {
        let existing_session = app
            .store
            .get_session(existing_session_id)
            .await
            .into_diagnostic()?
            .ok_or_else(|| {
                miette!(
                    "external {agent} session `{external_session_id}` is linked to missing bridge \
                     session `{existing_session_id}`; run `cb doctor --verbose` before retrying"
                )
            })?;
        if existing_session.project_id != app.project_id {
            let existing_root = app
                .store
                .project(&existing_session.project_id)
                .await
                .into_diagnostic()?
                .map_or_else(
                    || "<unknown project>".to_owned(),
                    |project| project.root.display().to_string(),
                );
            return Err(miette!(
                "external {agent} session `{external_session_id}` is already linked to \
                 `{existing_root}`, not the current project `{}`. Refusing to mix project \
                 contexts; the source session was not modified.",
                app.project.root.display()
            ));
        }
    }
    let session_id = existing.unwrap_or_else(BridgeSessionId::new);
    let mut sink = CollectingSink::default();
    adapter
        .import_session(
            ImportSessionRequest {
                bridge_session_id: session_id,
                external_session_id: external_session_id.clone(),
                source_path: source_path.clone(),
                full_context,
            },
            &mut sink,
        )
        .await
        .into_diagnostic()
        .wrap_err("external session import failed; the source session was not modified")?;

    let new_session = if existing.is_none() {
        let now = Utc::now();
        Some(BridgeSession {
            id: session_id,
            project_id: app.project_id.clone(),
            title,
            created_at: now,
            updated_at: now,
            status: SessionStatus::Paused,
            active_agent: Some(agent),
        })
    } else {
        None
    };
    let link = ExternalSessionLink {
        bridge_session_id: session_id,
        agent,
        external_session_id: external_session_id.clone(),
        source_path,
        imported_at: Utc::now(),
        last_synced_at: Some(Utc::now()),
        parser_version: if full_context {
            format!("{parser_version}-full-export")
        } else {
            parser_version
        },
    };
    let inserted = app
        .store
        .import_session_bundle(new_session.as_ref(), session_id, &sink.events, &link)
        .await
        .into_diagnostic()
        .wrap_err("could not transactionally store the imported session, events, and link")?;

    if app.json {
        print_json(&serde_json::json!({
            "bridge_session_id": session_id,
            "agent": agent,
            "external_session_id": external_session_id,
            "new_events": inserted,
            "already_imported": existing.is_some(),
            "full_context": full_context,
        }))?;
    } else {
        print_field("Bridge session", session_id);
        print_field("External session", external_session_id);
        print_field("New events", inserted);
    }
    Ok(0)
}

fn select_session(
    app: &App,
    agent: AgentKind,
    discovered: &[DiscoveredSession],
) -> miette::Result<Option<DiscoveredSession>> {
    if app.json {
        print_json(&discovered)?;
    } else {
        let rows = discovered
            .iter()
            .enumerate()
            .map(|(index, session)| {
                vec![
                    (index + 1).to_string(),
                    session.agent.to_string(),
                    session.external_session_id.to_string(),
                    session
                        .project_path
                        .as_ref()
                        .map_or_else(|| "-".to_owned(), |path| path.display().to_string()),
                    session.updated_at.to_rfc3339(),
                    session
                        .first_user_request
                        .as_deref()
                        .unwrap_or("-")
                        .chars()
                        .take(60)
                        .collect(),
                    session.approximate_event_count.to_string(),
                    if session.already_imported {
                        "imported"
                    } else {
                        "new"
                    }
                    .to_owned(),
                ]
            })
            .collect::<Vec<_>>();
        print_table(
            &[
                "#",
                "AGENT",
                "EXTERNAL ID",
                "PROJECT",
                "UPDATED",
                "FIRST REQUEST",
                "EVENTS",
                "STATUS",
            ],
            &rows,
        );
    }
    if discovered.is_empty() {
        return Err(miette!(
            "no safely discoverable {agent} sessions were found for this project",
        ));
    }
    if app.json || !std::io::stdin().is_terminal() {
        return Ok(None);
    }
    print!("Select a session to import [1-{}]: ", discovered.len());
    std::io::stdout().flush().into_diagnostic()?;
    let mut input = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut input)
        .into_diagnostic()?;
    let index: usize = input
        .trim()
        .parse()
        .into_diagnostic()
        .wrap_err("selection must be a number")?;
    discovered
        .get(index.saturating_sub(1))
        .cloned()
        .map(Some)
        .ok_or_else(|| miette!("selection {index} is out of range"))
}

#[derive(Default)]
struct CollectingSink {
    events: Vec<NewContextEvent>,
}

#[async_trait]
impl EventSink for CollectingSink {
    async fn push(&mut self, event: NewContextEvent) -> Result<(), AdapterError> {
        self.events.push(event);
        Ok(())
    }
}
