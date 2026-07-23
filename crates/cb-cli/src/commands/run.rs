use std::{
    io::{BufRead, IsTerminal, Write},
    path::PathBuf,
};

use cb_context::{HandoffBuilder, HandoffRequest};
use cb_core::{
    AgentKind, BridgeSessionId, Checkpoint, CheckpointId, ContextEventKind, ContextEventPayload,
    EventRepository, ExternalSessionLink, LaunchRequest, NewContextEvent, PostExitCaptureFailure,
    ProjectSummary, SecretScanner, Sensitivity, SessionRepository, SessionService, SessionStatus,
    SystemClock,
};
use cb_project::{FileChangeKind, ProjectSnapshot, diff_snapshots};
use chrono::Utc;
use miette::{IntoDiagnostic, WrapErr, miette};

use crate::{
    commands::App,
    output::{print_field, print_json, terminal_single_line},
};

pub async fn execute(app: &App, agent: AgentKind) -> miette::Result<i32> {
    if agent == AgentKind::OpenCode && !app.json && !app.quiet && std::io::stdin().is_terminal() {
        return opencode_session_deck(app).await;
    }
    launch_new(app, agent).await
}

async fn opencode_session_deck(app: &App) -> miette::Result<i32> {
    loop {
        println!();
        println!("   ,---.     context bridge · opencode · session deck");
        println!("  ( •‿~ )    choose where the work goes next");
        println!("  /|/|\\|\\");
        println!();
        println!("  [1] new       start a fresh OpenCode session");
        println!("  [2] import    choose an existing OpenCode session to move in");
        println!("  [3] continue  resume the latest bridged session in OpenCode");
        println!("  [4] sessions  inspect bridged sessions for this project");
        println!("  [q] quit");
        print!("\n  › ");
        std::io::stdout().flush().into_diagnostic()?;

        let mut choice = String::new();
        std::io::stdin()
            .lock()
            .read_line(&mut choice)
            .into_diagnostic()
            .wrap_err("could not read the OpenCode session-deck selection")?;
        match choice.trim().to_ascii_lowercase().as_str() {
            "1" | "new" | "n" => return launch_new(app, AgentKind::OpenCode).await,
            "2" | "import" | "i" => {
                super::import::execute(app, AgentKind::OpenCode, None, false).await?;
            }
            "3" | "continue" | "c" => {
                super::continue_cmd::execute(
                    app,
                    None,
                    None,
                    true,
                    AgentKind::OpenCode,
                    None,
                    false,
                )
                .await?;
            }
            "4" | "sessions" | "s" => {
                super::sessions::execute(app).await?;
            }
            "q" | "quit" | "exit" | "" => return Ok(0),
            _ => println!("  choose 1, 2, 3, 4, or q."),
        }
    }
}

async fn launch_new(app: &App, agent: AgentKind) -> miette::Result<i32> {
    print_launch_banner(app, agent);
    let session = SessionService::create(app.project_id.clone(), agent, &SystemClock);
    let session_id = session.id;
    app.store
        .create_session(&session)
        .await
        .into_diagnostic()
        .wrap_err("could not create a bridge session")?;

    let before = app
        .snapshot()
        .await
        .wrap_err("could not capture the initial project state")?;
    let initial = git_state_event(
        None,
        &before,
        format!("bridge:{session_id}:initial-git"),
        &app.scanner(),
    );
    app.store
        .append_events(session_id, &[initial])
        .await
        .into_diagnostic()?;

    print_launch_ready(app, agent, session_id, before.filesystem.files.len());

    let adapter = app.adapters.get(agent).into_diagnostic()?;
    let paths = runtime_paths(app, session_id, agent).await?;
    let running = match adapter
        .launch(LaunchRequest {
            bridge_session_id: session_id,
            project_root: app.project.root.clone(),
            bootstrap: None,
            event_sink_path: paths.event_sink.clone(),
            session_metadata_path: paths.session_metadata.clone(),
        })
        .await
    {
        Ok(running) => running,
        Err(error) => {
            app.store
                .update_session(session_id, SessionStatus::Failed, Some(agent))
                .await
                .into_diagnostic()?;
            return Err(error).into_diagnostic();
        }
    };
    let capture_failure_message = post_exit_capture_failure_message(
        agent,
        running.exit_code,
        &running.post_exit_capture_failures,
    );
    app.store
        .append_events(session_id, &running.events)
        .await
        .into_diagnostic()
        .wrap_err("could not import captured agent events")?;
    append_post_exit_capture_failures(
        app,
        session_id,
        agent,
        &running.post_exit_capture_failures,
        &format!("bridge:{session_id}:run-{agent}"),
    )
    .await?;

    let after = app
        .snapshot()
        .await
        .wrap_err("could not reconcile the project after agent exit")?;
    append_reconciliation(
        app,
        session_id,
        Some(agent),
        &before,
        &after,
        &format!("bridge:{session_id}:run-{agent}"),
    )
    .await?;

    if let Some(external_session_id) = running.external_session_id.clone() {
        app.store
            .link_external_session(&ExternalSessionLink {
                bridge_session_id: session_id,
                agent,
                external_session_id,
                source_path: Some(paths.event_sink.clone()),
                imported_at: Utc::now(),
                last_synced_at: Some(Utc::now()),
                parser_version: running.parser_version.clone(),
            })
            .await
            .into_diagnostic()?;
    }

    add_automatic_checkpoint(app, session_id, agent, running.exit_code).await?;
    let final_status = if running.exit_code == 0 && capture_failure_message.is_none() {
        SessionStatus::Paused
    } else {
        SessionStatus::Failed
    };
    app.store
        .update_session(session_id, final_status, Some(agent))
        .await
        .into_diagnostic()?;
    save_summary(
        app,
        session_id,
        agent,
        agent,
        &after,
        app.config.general.context_budget,
    )
    .await?;

    if app.json {
        print_json(&serde_json::json!({
            "bridge_session_id": session_id,
            "agent": agent,
            "external_session_id": running.external_session_id,
            "exit_code": running.exit_code,
            "post_exit_capture_failures": running.post_exit_capture_failures,
        }))?;
    } else {
        print_field("Bridge session", session_id);
        print_field("Agent", agent);
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

fn show_launch_status(app: &App) -> bool {
    !app.json && !app.quiet && std::io::stdout().is_terminal()
}

fn print_launch_banner(app: &App, agent: AgentKind) {
    if !show_launch_status(app) {
        return;
    }
    println!();
    println!("   ,---.     context bridge · {agent} · new run");
    println!("  ( •‿~ )    “what are we working on?”");
    println!(
        "  /|/|\\|\\    scanning project · {}",
        terminal_single_line(&app.project.root.display().to_string())
    );
    println!("              source files only · dependencies and agent state stay out");
    println!();
    let _ = std::io::stdout().flush();
}

fn print_launch_ready(app: &App, agent: AgentKind, session_id: BridgeSessionId, file_count: usize) {
    if !show_launch_status(app) {
        return;
    }
    println!("  ✓ snapshot ready · {file_count} files · bridge {session_id}");
    println!("  ↳ opening {agent} in this terminal now");
    println!("  ↳ exit {agent} when you are done; Context Bridge will reconcile changes.");
    println!();
    let _ = std::io::stdout().flush();
}

pub(crate) struct RuntimePaths {
    pub event_sink: PathBuf,
    pub session_metadata: PathBuf,
}

pub(crate) async fn runtime_paths(
    app: &App,
    session_id: BridgeSessionId,
    agent: AgentKind,
) -> miette::Result<RuntimePaths> {
    let directory = app
        .data_dir
        .join("runtime")
        .join(session_id.to_string())
        .join(format!("{agent}-{}", Utc::now().timestamp_millis()));
    tokio::fs::create_dir_all(&directory)
        .await
        .into_diagnostic()
        .wrap_err_with(|| {
            format!(
                "could not create runtime directory `{}`",
                directory.display()
            )
        })?;
    set_directory_permissions(&directory)?;
    Ok(RuntimePaths {
        event_sink: directory.join("events.jsonl"),
        session_metadata: directory.join("session.json"),
    })
}

pub(crate) async fn append_reconciliation(
    app: &App,
    session_id: BridgeSessionId,
    source_agent: Option<AgentKind>,
    before: &ProjectSnapshot,
    after: &ProjectSnapshot,
    key_prefix: &str,
) -> miette::Result<()> {
    let scanner = app.scanner();
    let path_policy = app.path_policy()?;
    let mut events = Vec::new();
    for (index, change) in diff_snapshots(before, after).into_iter().enumerate() {
        let (kind, previous_path) = match change.kind {
            FileChangeKind::Created => (ContextEventKind::FileCreated, None),
            FileChangeKind::Modified => (ContextEventKind::FileModified, None),
            FileChangeKind::Deleted => (ContextEventKind::FileDeleted, None),
            FileChangeKind::Moved { from } => (ContextEventKind::FileMoved, Some(from)),
        };
        events.push(NewContextEvent {
            source_agent,
            external_event_id: None,
            timestamp: after.captured_at,
            kind,
            payload: ContextEventPayload::File {
                path: change.path.clone(),
                previous_path,
                summary: Some("Detected by before/after filesystem reconciliation".to_owned()),
            },
            sensitivity: if path_policy.is_excluded(&change.path) {
                Sensitivity::Excluded
            } else {
                Sensitivity::Normal
            },
            import_metadata: None,
            parent_event_id: None,
            metadata: serde_json::Value::Null,
            import_key: format!("{key_prefix}:file:{index}:{}", change.path.display()),
        });
    }
    events.push(git_state_event(
        source_agent,
        after,
        format!("{key_prefix}:git-state"),
        &scanner,
    ));
    let diff = format!(
        "## Staged diff\n{}\n## Unstaged diff\n{}",
        after.git.staged_diff, after.git.unstaged_diff
    );
    events.push(NewContextEvent {
        source_agent,
        external_event_id: None,
        timestamp: after.captured_at,
        kind: ContextEventKind::GitDiff,
        payload: ContextEventPayload::GitDiff {
            diff: diff.clone(),
            truncated: after.git.truncated,
        },
        sensitivity: scanner.classify(&diff),
        import_metadata: None,
        parent_event_id: None,
        metadata: serde_json::Value::Null,
        import_key: format!("{key_prefix}:git-diff"),
    });
    app.store
        .append_events(session_id, &events)
        .await
        .into_diagnostic()
        .wrap_err("could not persist project reconciliation")?;
    Ok(())
}

pub(crate) async fn append_post_exit_capture_failures(
    app: &App,
    session_id: BridgeSessionId,
    agent: AgentKind,
    failures: &[PostExitCaptureFailure],
    key_prefix: &str,
) -> miette::Result<()> {
    if failures.is_empty() {
        return Ok(());
    }
    let events = failures
        .iter()
        .enumerate()
        .map(|(index, failure)| NewContextEvent {
            source_agent: Some(agent),
            external_event_id: None,
            timestamp: Utc::now(),
            kind: ContextEventKind::Error,
            payload: ContextEventPayload::Error {
                message: format!(
                    "Could not capture {} after {agent} exited: {}",
                    failure.stage, failure.details
                ),
                context: Some("post-exit agent capture".to_owned()),
                resolved: false,
            },
            sensitivity: Sensitivity::Normal,
            import_metadata: None,
            parent_event_id: None,
            metadata: serde_json::json!({ "capture_stage": failure.stage }),
            import_key: format!("{key_prefix}:post-exit-capture-error:{index}"),
        })
        .collect::<Vec<_>>();
    app.store
        .append_events(session_id, &events)
        .await
        .into_diagnostic()
        .wrap_err("could not persist post-exit capture failures")?;
    Ok(())
}

#[must_use]
pub(crate) fn post_exit_capture_failure_message(
    agent: AgentKind,
    exit_code: i32,
    failures: &[PostExitCaptureFailure],
) -> Option<String> {
    (!failures.is_empty()).then(|| {
        let details = failures
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ");
        format!(
            "{agent} exited with status {exit_code}, but Context Bridge could not capture \
             all post-exit artifacts ({details}). Project reconciliation was persisted and the \
             bridge session was finalized as failed."
        )
    })
}

pub(crate) async fn save_summary(
    app: &App,
    session_id: BridgeSessionId,
    source_agent: AgentKind,
    target_agent: AgentKind,
    snapshot: &ProjectSnapshot,
    budget: usize,
) -> miette::Result<cb_core::HandoffPackage> {
    let events = app
        .store
        .events(session_id)
        .await
        .into_diagnostic()
        .wrap_err("could not read canonical events")?;
    let scanner = app.scanner();
    let path_policy = app.path_policy()?;
    let package = HandoffBuilder::new(&scanner, &path_policy)
        .build(HandoffRequest {
            session_id,
            source_agent,
            target_agent,
            project: ProjectSummary {
                id: app.project_id.to_string(),
                root: app.project.root.clone(),
            },
            events: &events,
            git: (&snapshot.git).into(),
            budget,
        })
        .into_diagnostic()
        .wrap_err("could not build a deterministic handoff summary")?;
    app.store
        .save_handoff(&package)
        .await
        .into_diagnostic()
        .wrap_err("could not save the derived handoff package")?;
    Ok(package)
}

fn git_state_event(
    source_agent: Option<AgentKind>,
    snapshot: &ProjectSnapshot,
    import_key: String,
    scanner: &dyn SecretScanner,
) -> NewContextEvent {
    let payload = ContextEventPayload::GitState {
        branch: snapshot.git.branch.clone(),
        head: snapshot.git.head.clone(),
        status: snapshot.git.status.clone(),
        staged_diff: snapshot.git.staged_diff.clone(),
        unstaged_diff: snapshot.git.unstaged_diff.clone(),
        untracked_files: snapshot.git.untracked_files.clone(),
        filesystem_file_count: snapshot.filesystem.files.len(),
        filesystem_fingerprint: snapshot.filesystem.fingerprint(),
    };
    let serialized = serde_json::to_string(&payload).unwrap_or_default();
    NewContextEvent {
        source_agent,
        external_event_id: None,
        timestamp: snapshot.captured_at,
        kind: ContextEventKind::GitState,
        payload,
        sensitivity: scanner.classify(&serialized),
        import_metadata: None,
        parent_event_id: None,
        metadata: serde_json::Value::Null,
        import_key,
    }
}

async fn add_automatic_checkpoint(
    app: &App,
    session_id: BridgeSessionId,
    agent: AgentKind,
    exit_code: i32,
) -> miette::Result<()> {
    let note = Some(format!(
        "{agent} exited with status {exit_code}; project state reconciled."
    ));
    let inserted = app
        .store
        .append_events(
            session_id,
            &[NewContextEvent {
                source_agent: Some(agent),
                external_event_id: None,
                timestamp: Utc::now(),
                kind: ContextEventKind::Checkpoint,
                payload: ContextEventPayload::Checkpoint {
                    note: note.clone(),
                    completed_work: Vec::new(),
                    pending_tasks: Vec::new(),
                    recommended_next_action: None,
                },
                sensitivity: Sensitivity::Normal,
                import_metadata: None,
                parent_event_id: None,
                metadata: serde_json::Value::Null,
                import_key: format!("bridge:{session_id}:automatic-checkpoint:{agent}"),
            }],
        )
        .await
        .into_diagnostic()?;
    if let Some(event) = inserted.first() {
        app.store
            .create_checkpoint(&Checkpoint {
                id: CheckpointId::new(),
                bridge_session_id: session_id,
                note,
                created_at: event.timestamp,
                event_sequence: event.sequence,
            })
            .await
            .into_diagnostic()?;
    }
    Ok(())
}

#[cfg(unix)]
fn set_directory_permissions(path: &std::path::Path) -> miette::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .into_diagnostic()
        .wrap_err_with(|| format!("could not restrict runtime directory `{}`", path.display()))
}

#[cfg(not(unix))]
fn set_directory_permissions(_path: &std::path::Path) -> miette::Result<()> {
    Ok(())
}
