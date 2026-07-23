use cb_core::{
    Checkpoint, CheckpointId, ContextEventKind, ContextEventPayload, EventRepository,
    NewContextEvent, SecretScanner, SessionRepository,
};
use chrono::Utc;
use miette::{IntoDiagnostic, WrapErr, miette};

use crate::{
    commands::{App, run::append_reconciliation},
    output::{print_field, print_json},
};

pub async fn execute(app: &App, note: Option<String>) -> miette::Result<i32> {
    let session = app
        .store
        .last_session(Some(&app.project_id))
        .await
        .into_diagnostic()?
        .ok_or_else(|| miette!("this project has no bridge session to checkpoint"))?;
    let snapshot = app
        .snapshot()
        .await
        .wrap_err("could not capture project state for the checkpoint")?;
    let unique = Utc::now().timestamp_nanos_opt().unwrap_or_default();
    append_reconciliation(
        app,
        session.id,
        session.active_agent,
        &snapshot,
        &snapshot,
        &format!("bridge:{}:manual-checkpoint:{unique}", session.id),
    )
    .await?;
    let sensitivity = note.as_ref().map_or(cb_core::Sensitivity::Normal, |note| {
        app.scanner().classify(note)
    });
    let event = NewContextEvent {
        source_agent: session.active_agent,
        external_event_id: None,
        timestamp: Utc::now(),
        kind: ContextEventKind::Checkpoint,
        payload: ContextEventPayload::Checkpoint {
            note: note.clone(),
            completed_work: Vec::new(),
            pending_tasks: Vec::new(),
            recommended_next_action: None,
        },
        sensitivity,
        import_metadata: None,
        parent_event_id: None,
        metadata: serde_json::Value::Null,
        import_key: format!("bridge:{}:manual-checkpoint-event:{unique}", session.id),
    };
    let inserted = app
        .store
        .append_events(session.id, &[event])
        .await
        .into_diagnostic()?;
    let inserted = inserted
        .first()
        .ok_or_else(|| miette!("checkpoint was unexpectedly deduplicated"))?;
    let checkpoint = Checkpoint {
        id: CheckpointId::new(),
        bridge_session_id: session.id,
        note,
        created_at: inserted.timestamp,
        event_sequence: inserted.sequence,
    };
    app.store
        .create_checkpoint(&checkpoint)
        .await
        .into_diagnostic()?;
    if app.json {
        print_json(&checkpoint)?;
    } else {
        print_field("Checkpoint", checkpoint.id);
        print_field("Bridge session", checkpoint.bridge_session_id);
        print_field("Event sequence", checkpoint.event_sequence);
    }
    Ok(0)
}
