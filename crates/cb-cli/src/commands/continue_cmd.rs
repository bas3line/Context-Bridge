use async_trait::async_trait;
use cb_context::renderers::{render_claude, render_codex, render_opencode};
use cb_core::{
    AdapterError, AgentKind, BridgeSession, BridgeSessionId, ContextEventKind, ContextEventPayload,
    EventRepository, EventSink, ExternalSessionLink, LaunchRequest, NewContextEvent, ResumeRequest,
    Sensitivity, SessionRepository, SessionStatus,
};
use chrono::Utc;
use miette::{IntoDiagnostic, WrapErr, miette};

use crate::{
    commands::{
        App,
        run::{
            append_post_exit_capture_failures, append_reconciliation,
            post_exit_capture_failure_message, runtime_paths, save_summary,
        },
    },
    output::{print_field, print_json, terminal_safe},
};

pub async fn execute(
    app: &App,
    from: Option<AgentKind>,
    session_id: Option<BridgeSessionId>,
    _last: bool,
    target: AgentKind,
    budget: Option<usize>,
    preview: bool,
) -> miette::Result<i32> {
    let session = resolve_session(app, session_id, from).await?;
    let source = resolve_source_agent(app, &session, from).await?;
    refresh_source(app, session.id, source).await?;

    let before = app
        .snapshot()
        .await
        .wrap_err("could not refresh project state before handoff")?;
    append_reconciliation(
        app,
        session.id,
        Some(source),
        &before,
        &before,
        &format!(
            "bridge:{}:pre-handoff:{}",
            session.id,
            Utc::now().timestamp_millis()
        ),
    )
    .await?;
    let package = save_summary(
        app,
        session.id,
        source,
        target,
        &before,
        budget.unwrap_or(app.config.general.context_budget),
    )
    .await?;
    let rendered = match target {
        AgentKind::Codex => render_codex(&package),
        AgentKind::ClaudeCode => render_claude(&package),
        AgentKind::OpenCode => render_opencode(&package),
    };
    if preview || app.config.general.preview_before_handoff {
        let terminal_rendered = terminal_safe(&rendered);
        if preview {
            println!("{terminal_rendered}");
            return Ok(0);
        }
        eprintln!(
            "--- Context Bridge handoff preview ---\n{terminal_rendered}\n--- End preview ---"
        );
    }

    let handoff_event = NewContextEvent {
        source_agent: Some(source),
        external_event_id: None,
        timestamp: Utc::now(),
        kind: ContextEventKind::Handoff,
        payload: ContextEventPayload::Handoff {
            target_agent: target,
            handoff_id: package.id.to_string(),
        },
        sensitivity: Sensitivity::Normal,
        import_metadata: None,
        parent_event_id: None,
        metadata: serde_json::Value::Null,
        import_key: format!("bridge:{}:handoff:{}", session.id, package.id),
    };
    app.store
        .append_events(session.id, &[handoff_event])
        .await
        .into_diagnostic()?;

    let adapter = app.adapters.get(target).into_diagnostic()?;
    let existing_target = app
        .store
        .external_links(session.id)
        .await
        .into_diagnostic()?
        .into_iter()
        .rev()
        .find(|link| link.agent == target);
    let capabilities = adapter.capabilities().await.into_diagnostic()?;
    if !app.config.test_mode
        && !capabilities.initial_prompt_argument
        && !capabilities.stdin_prompt
        && !capabilities.server_api
    {
        let path = write_manual_handoff(app, session.id, package.id, &rendered).await?;
        if app.json {
            print_json(&serde_json::json!({
                "bridge_session_id": session.id,
                "from": source,
                "to": target,
                "launched": false,
                "manual_handoff_path": path,
                "reason": "target compatibility profile has no verified context injection",
            }))?;
        } else {
            print_field("Bridge session", session.id);
            print_field("Manual handoff", path.display());
            eprintln!(
                "{target} was not launched because its detected profile has no verified \
                 context-injection interface."
            );
        }
        return Ok(0);
    }
    app.store
        .update_session(session.id, SessionStatus::Active, Some(target))
        .await
        .into_diagnostic()?;
    let paths = runtime_paths(app, session.id, target).await?;
    let running = if let Some(link) = existing_target.filter(|_| capabilities.native_resume) {
        adapter
            .resume(ResumeRequest {
                bridge_session_id: session.id,
                external_session_id: link.external_session_id,
                project_root: app.project.root.clone(),
                bootstrap: rendered,
                event_sink_path: paths.event_sink.clone(),
                session_metadata_path: paths.session_metadata.clone(),
            })
            .await
    } else {
        adapter
            .launch(LaunchRequest {
                bridge_session_id: session.id,
                project_root: app.project.root.clone(),
                bootstrap: Some(rendered),
                event_sink_path: paths.event_sink.clone(),
                session_metadata_path: paths.session_metadata.clone(),
            })
            .await
    };
    let running = match running {
        Ok(running) => running,
        Err(error) => {
            if target == AgentKind::ClaudeCode && error.to_string().contains("already in use") {
                app.store
                    .update_session(session.id, SessionStatus::Paused, Some(source))
                    .await
                    .into_diagnostic()?;
                return Err(miette!(
                    "the linked Claude session is still open elsewhere. Exit that Claude terminal first, then retry this command; the bridge session remains paused."
                ));
            }
            app.store
                .update_session(session.id, SessionStatus::Failed, Some(target))
                .await
                .into_diagnostic()?;
            return Err(error).into_diagnostic();
        }
    };
    let capture_failure_message = post_exit_capture_failure_message(
        target,
        running.exit_code,
        &running.post_exit_capture_failures,
    );
    app.store
        .append_events(session.id, &running.events)
        .await
        .into_diagnostic()
        .wrap_err("could not import target-agent events")?;
    append_post_exit_capture_failures(
        app,
        session.id,
        target,
        &running.post_exit_capture_failures,
        &format!("bridge:{}:continue-{}:{}", session.id, target, package.id),
    )
    .await?;
    let after = app
        .snapshot()
        .await
        .wrap_err("could not reconcile target-agent changes")?;
    append_reconciliation(
        app,
        session.id,
        Some(target),
        &before,
        &after,
        &format!("bridge:{}:continue-{}:{}", session.id, target, package.id),
    )
    .await?;
    if let Some(external_session_id) = running.external_session_id.clone() {
        app.store
            .link_external_session(&ExternalSessionLink {
                bridge_session_id: session.id,
                agent: target,
                external_session_id,
                source_path: Some(paths.event_sink),
                imported_at: Utc::now(),
                last_synced_at: Some(Utc::now()),
                parser_version: running.parser_version.clone(),
            })
            .await
            .into_diagnostic()?;
    }
    let checkpoint = NewContextEvent {
        source_agent: Some(target),
        external_event_id: None,
        timestamp: Utc::now(),
        kind: ContextEventKind::Checkpoint,
        payload: ContextEventPayload::Checkpoint {
            note: Some(format!(
                "{target} exited with status {}; combined context refreshed.",
                running.exit_code
            )),
            completed_work: Vec::new(),
            pending_tasks: Vec::new(),
            recommended_next_action: None,
        },
        sensitivity: Sensitivity::Normal,
        import_metadata: None,
        parent_event_id: None,
        metadata: serde_json::Value::Null,
        import_key: format!("bridge:{}:continue-checkpoint:{}", session.id, package.id),
    };
    app.store
        .append_events(session.id, &[checkpoint])
        .await
        .into_diagnostic()?;
    let final_status = if running.exit_code == 0 && capture_failure_message.is_none() {
        SessionStatus::Paused
    } else {
        SessionStatus::Failed
    };
    app.store
        .update_session(session.id, final_status, Some(target))
        .await
        .into_diagnostic()?;
    save_summary(
        app,
        session.id,
        target,
        target,
        &after,
        budget.unwrap_or(app.config.general.context_budget),
    )
    .await?;

    if app.json {
        print_json(&serde_json::json!({
            "bridge_session_id": session.id,
            "from": source,
            "to": target,
            "external_session_id": running.external_session_id,
            "exit_code": running.exit_code,
            "post_exit_capture_failures": running.post_exit_capture_failures,
        }))?;
    } else {
        print_field("Bridge session", session.id);
        print_field("Continued from", source);
        print_field("Continued to", target);
        if let Some(external_id) = running.external_session_id {
            print_field("External session", external_id);
        }
        print_field("Exit code", running.exit_code);
        if let Some(message) = &capture_failure_message {
            print_field("Capture failure", message);
        }
    }
    if let Some(message) = capture_failure_message
        && running.exit_code == 0
    {
        return Err(miette!(message));
    }
    Ok(running.exit_code)
}

async fn resolve_session(
    app: &App,
    requested: Option<BridgeSessionId>,
    from: Option<AgentKind>,
) -> miette::Result<BridgeSession> {
    if let Some(id) = requested {
        let session = app
            .store
            .get_session(id)
            .await
            .into_diagnostic()?
            .ok_or_else(|| miette!("bridge session `{id}` was not found"))?;
        if session.project_id != app.project_id {
            let root = app
                .store
                .project(&session.project_id)
                .await
                .into_diagnostic()?
                .map_or_else(
                    || "<unknown>".to_owned(),
                    |project| project.root.display().to_string(),
                );
            return Err(miette!(
                "bridge session `{id}` belongs to `{root}`, not the current project. \
                 Retry with `--project {root}`."
            ));
        }
        return Ok(session);
    }
    let sessions = app
        .store
        .list_sessions(Some(&app.project_id))
        .await
        .into_diagnostic()?;
    for session in sessions {
        if let Some(from) = from {
            let links = app
                .store
                .external_links(session.id)
                .await
                .into_diagnostic()?;
            if session.active_agent != Some(from) && !links.iter().any(|link| link.agent == from) {
                continue;
            }
        }
        return Ok(session);
    }
    Err(miette!(
        "no bridge session matches this project and source agent; run or import a session first"
    ))
}

async fn resolve_source_agent(
    app: &App,
    session: &BridgeSession,
    requested: Option<AgentKind>,
) -> miette::Result<AgentKind> {
    if let Some(requested) = requested {
        return Ok(requested);
    }
    if let Some(active) = session.active_agent {
        return Ok(active);
    }
    app.store
        .external_links(session.id)
        .await
        .into_diagnostic()?
        .last()
        .map(|link| link.agent)
        .ok_or_else(|| miette!("session `{}` has no source agent link", session.id))
}

async fn refresh_source(
    app: &App,
    session_id: BridgeSessionId,
    source: AgentKind,
) -> miette::Result<()> {
    let link = app
        .store
        .external_links(session_id)
        .await
        .into_diagnostic()?
        .into_iter()
        .rev()
        .find(|link| link.agent == source);
    let Some(link) = link else {
        return Ok(());
    };
    let adapter = app.adapters.get(source).into_diagnostic()?;
    let mut sink = CollectingSink::default();
    match adapter.refresh(&link, &mut sink).await {
        Ok(_) => {
            app.store
                .append_events(session_id, &sink.events)
                .await
                .into_diagnostic()
                .wrap_err("could not persist refreshed source events")?;
        }
        Err(AdapterError::UnsupportedVersion { details, .. }) => {
            tracing::warn!(%details, "source refresh is unavailable; using stored canonical events");
        }
        Err(error) => return Err(error).into_diagnostic(),
    }
    Ok(())
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

async fn write_manual_handoff(
    app: &App,
    session_id: BridgeSessionId,
    handoff_id: cb_core::HandoffId,
    rendered: &str,
) -> miette::Result<std::path::PathBuf> {
    let directory = app.data_dir.join("handoffs").join(session_id.to_string());
    tokio::fs::create_dir_all(&directory)
        .await
        .into_diagnostic()
        .wrap_err_with(|| {
            format!(
                "could not create manual handoff directory `{}`",
                directory.display()
            )
        })?;
    let path = directory.join(format!("{handoff_id}.md"));
    tokio::fs::write(&path, rendered)
        .await
        .into_diagnostic()
        .wrap_err_with(|| format!("could not write manual handoff `{}`", path.display()))?;
    restrict_path(&directory, true)?;
    restrict_path(&path, false)?;
    Ok(path)
}

#[cfg(unix)]
fn restrict_path(path: &std::path::Path, directory: bool) -> miette::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = if directory { 0o700 } else { 0o600 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .into_diagnostic()
        .wrap_err_with(|| format!("could not restrict `{}`", path.display()))
}

#[cfg(not(unix))]
fn restrict_path(_path: &std::path::Path, _directory: bool) -> miette::Result<()> {
    Ok(())
}
