use std::sync::Arc;

use cb_core::{
    AgentKind, BridgeSession, BridgeSessionId, ContextEventKind, ContextEventPayload,
    EventRepository, ExternalSessionId, ExternalSessionLink, NewContextEvent, ProjectId,
    ProjectRecord, Sensitivity, SessionRepository, SessionStatus,
};
use cb_storage::SqliteStore;
use chrono::Utc;
use serde_json::Value;
use tokio::{sync::Barrier, task::JoinSet};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_appends_and_imports_allocate_contiguous_sequences() {
    const APPENDERS: usize = 8;
    const IMPORTERS: usize = 8;

    let directory = tempfile::tempdir().expect("temporary storage directory");
    let database_path = directory.path().join("private").join("bridge.db");
    let store = SqliteStore::open(&database_path).await.expect("open store");
    // Use a distinct connection pool too: the serialization guarantee must come from SQLite's
    // writer lock, not incidental coordination inside a single `SqliteStore` instance.
    let peer_store = SqliteStore::open(&database_path)
        .await
        .expect("open peer store");
    let now = Utc::now();
    let project_id = ProjectId::new("concurrent-project").expect("project id");
    store
        .upsert_project(&ProjectRecord {
            id: project_id.clone(),
            root: directory.path().to_path_buf(),
            is_git: false,
            created_at: now,
            updated_at: now,
        })
        .await
        .expect("project");
    let session_id = BridgeSessionId::new();
    store
        .create_session(&BridgeSession {
            id: session_id,
            project_id,
            title: None,
            created_at: now,
            updated_at: now,
            status: SessionStatus::Active,
            active_agent: None,
        })
        .await
        .expect("session");

    let barrier = Arc::new(Barrier::new(APPENDERS + IMPORTERS));
    let mut writes = JoinSet::new();

    for index in 0..APPENDERS {
        let store = if index.is_multiple_of(2) {
            store.clone()
        } else {
            peer_store.clone()
        };
        let barrier = Arc::clone(&barrier);
        writes.spawn(async move {
            barrier.wait().await;
            store
                .append_events(session_id, &[event("append", index)])
                .await
                .map(|events| events.len())
        });
    }

    for index in 0..IMPORTERS {
        let store = if index.is_multiple_of(2) {
            store.clone()
        } else {
            peer_store.clone()
        };
        let barrier = Arc::clone(&barrier);
        writes.spawn(async move {
            barrier.wait().await;
            store
                .import_session_bundle(
                    None,
                    session_id,
                    &[event("import", index)],
                    &ExternalSessionLink {
                        bridge_session_id: session_id,
                        agent: AgentKind::OpenCode,
                        external_session_id: ExternalSessionId::new(format!(
                            "concurrent-import-{index}"
                        ))
                        .expect("external session id"),
                        source_path: None,
                        imported_at: Utc::now(),
                        last_synced_at: None,
                        parser_version: "test-v1".to_owned(),
                    },
                )
                .await
        });
    }

    while let Some(write) = writes.join_next().await {
        assert_eq!(
            write
                .expect("write task must not panic")
                .expect("concurrent write"),
            1,
            "each unique event must be inserted exactly once"
        );
    }

    let events = store.events(session_id).await.expect("events");
    assert_eq!(events.len(), APPENDERS + IMPORTERS);
    assert_eq!(
        events
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        (1..=i64::try_from(APPENDERS + IMPORTERS).expect("event count fits i64"))
            .collect::<Vec<_>>(),
        "one writer reservation must make per-session sequences gap-free and monotonic"
    );
    assert_eq!(
        store
            .external_links(session_id)
            .await
            .expect("external links")
            .len(),
        IMPORTERS
    );
    store.health_check().await.expect("healthy database");
}

fn event(origin: &str, index: usize) -> NewContextEvent {
    NewContextEvent {
        source_agent: Some(AgentKind::Codex),
        external_event_id: Some(format!("{origin}-external-{index}")),
        timestamp: Utc::now(),
        kind: ContextEventKind::UserMessage,
        payload: ContextEventPayload::Message {
            content: format!("{origin} event {index}"),
        },
        sensitivity: Sensitivity::Normal,
        import_metadata: None,
        parent_event_id: None,
        metadata: Value::Null,
        import_key: format!("{origin}-import-{index}"),
    }
}
