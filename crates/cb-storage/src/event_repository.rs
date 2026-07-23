use std::str::FromStr;

use async_trait::async_trait;
use cb_core::{
    AgentKind, BridgeSessionId, ContextEvent, ContextEventKind, EventId, EventRepository,
    ImportMetadata, NewContextEvent, Sensitivity,
};
use sqlx::Row;

use crate::{SqliteStore, StorageError, sqlite::parse_datetime};

#[async_trait]
impl EventRepository for SqliteStore {
    type Error = StorageError;

    async fn append_events(
        &self,
        session_id: BridgeSessionId,
        events: &[NewContextEvent],
    ) -> Result<Vec<ContextEvent>, Self::Error> {
        // Sequence allocation is a read-modify-write operation. Taking SQLite's writer
        // reservation before reading MAX(sequence) serializes concurrent appenders (including
        // other processes) and prevents two transactions from choosing the same sequence.
        let mut transaction = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(StorageError::Sqlx)?;
        let mut sequence: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(sequence), 0)
             FROM context_events WHERE bridge_session_id = ?",
        )
        .bind(session_id.to_string())
        .fetch_one(&mut *transaction)
        .await
        .map_err(StorageError::Sqlx)?;
        let mut inserted = Vec::with_capacity(events.len());

        for event in events {
            sequence += 1;
            let id = EventId::new();
            let content_hash = event.content_hash()?;
            let payload_json = serde_json::to_string(&event.payload)?;
            let metadata_json = serde_json::to_string(&event.metadata)?;
            let import_metadata_json = event
                .import_metadata
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?;
            let result = sqlx::query(
                "INSERT OR IGNORE INTO context_events
                 (id, bridge_session_id, source_agent, external_event_id, sequence,
                  timestamp, kind, payload_json, content_hash, sensitivity,
                  import_metadata_json, parent_event_id, metadata_json, import_key)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(id.to_string())
            .bind(session_id.to_string())
            .bind(event.source_agent.map(|agent| agent.to_string()))
            .bind(&event.external_event_id)
            .bind(sequence)
            .bind(event.timestamp.to_rfc3339())
            .bind(event_kind_string(event.kind))
            .bind(payload_json)
            .bind(&content_hash)
            .bind(sensitivity_string(event.sensitivity))
            .bind(import_metadata_json)
            .bind(event.parent_event_id.map(|parent| parent.to_string()))
            .bind(metadata_json)
            .bind(&event.import_key)
            .execute(&mut *transaction)
            .await
            .map_err(StorageError::Sqlx)?;

            if result.rows_affected() == 0 {
                sequence -= 1;
                continue;
            }

            inserted.push(ContextEvent {
                id,
                bridge_session_id: session_id,
                source_agent: event.source_agent,
                external_event_id: event.external_event_id.clone(),
                sequence,
                timestamp: event.timestamp,
                kind: event.kind,
                payload: event.payload.clone(),
                content_hash,
                sensitivity: event.sensitivity,
                import_metadata: event.import_metadata.clone(),
                parent_event_id: event.parent_event_id,
                metadata: event.metadata.clone(),
            });
        }

        sqlx::query("UPDATE bridge_sessions SET updated_at = ? WHERE id = ?")
            .bind(chrono::Utc::now().to_rfc3339())
            .bind(session_id.to_string())
            .execute(&mut *transaction)
            .await
            .map_err(StorageError::Sqlx)?;
        transaction.commit().await.map_err(StorageError::Sqlx)?;
        Ok(inserted)
    }

    async fn events(&self, session_id: BridgeSessionId) -> Result<Vec<ContextEvent>, Self::Error> {
        let rows = sqlx::query(
            "SELECT id, bridge_session_id, source_agent, external_event_id, sequence,
                    timestamp, kind, payload_json, content_hash, sensitivity,
                    import_metadata_json, parent_event_id, metadata_json
             FROM context_events WHERE bridge_session_id = ? ORDER BY sequence",
        )
        .bind(session_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(StorageError::Sqlx)?;
        rows.into_iter().map(row_to_event).collect()
    }

    async fn event_count(&self, session_id: BridgeSessionId) -> Result<u64, Self::Error> {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM context_events WHERE bridge_session_id = ?")
                .bind(session_id.to_string())
                .fetch_one(&self.pool)
                .await
                .map_err(StorageError::Sqlx)?;
        u64::try_from(count)
            .map_err(|_| StorageError::Integrity(format!("negative event count {count}")))
    }
}

fn row_to_event(row: sqlx::sqlite::SqliteRow) -> Result<ContextEvent, StorageError> {
    Ok(ContextEvent {
        id: row.try_get::<String, _>("id")?.parse::<EventId>()?,
        bridge_session_id: row
            .try_get::<String, _>("bridge_session_id")?
            .parse::<BridgeSessionId>()?,
        source_agent: row
            .try_get::<Option<String>, _>("source_agent")?
            .map(|value| {
                AgentKind::from_str(&value)
                    .map_err(|error| StorageError::Integrity(error.to_string()))
            })
            .transpose()?,
        external_event_id: row.try_get("external_event_id")?,
        sequence: row.try_get("sequence")?,
        timestamp: parse_datetime(&row.try_get::<String, _>("timestamp")?)?,
        kind: parse_event_kind(&row.try_get::<String, _>("kind")?)?,
        payload: serde_json::from_str(&row.try_get::<String, _>("payload_json")?)?,
        content_hash: row.try_get("content_hash")?,
        sensitivity: parse_sensitivity(&row.try_get::<String, _>("sensitivity")?)?,
        import_metadata: row
            .try_get::<Option<String>, _>("import_metadata_json")?
            .map(|value| serde_json::from_str::<ImportMetadata>(&value))
            .transpose()?,
        parent_event_id: row
            .try_get::<Option<String>, _>("parent_event_id")?
            .map(|value| value.parse::<EventId>())
            .transpose()?,
        metadata: serde_json::from_str(&row.try_get::<String, _>("metadata_json")?)?,
    })
}

pub(crate) const fn event_kind_string(kind: ContextEventKind) -> &'static str {
    match kind {
        ContextEventKind::UserMessage => "user_message",
        ContextEventKind::AssistantMessage => "assistant_message",
        ContextEventKind::SystemMessage => "system_message",
        ContextEventKind::ToolCall => "tool_call",
        ContextEventKind::ToolResult => "tool_result",
        ContextEventKind::CommandExecuted => "command_executed",
        ContextEventKind::FileRead => "file_read",
        ContextEventKind::FileCreated => "file_created",
        ContextEventKind::FileModified => "file_modified",
        ContextEventKind::FileDeleted => "file_deleted",
        ContextEventKind::FileMoved => "file_moved",
        ContextEventKind::GitState => "git_state",
        ContextEventKind::GitDiff => "git_diff",
        ContextEventKind::TestRun => "test_run",
        ContextEventKind::Decision => "decision",
        ContextEventKind::Assumption => "assumption",
        ContextEventKind::Error => "error",
        ContextEventKind::Checkpoint => "checkpoint",
        ContextEventKind::Handoff => "handoff",
    }
}

fn parse_event_kind(value: &str) -> Result<ContextEventKind, StorageError> {
    match value {
        "user_message" => Ok(ContextEventKind::UserMessage),
        "assistant_message" => Ok(ContextEventKind::AssistantMessage),
        "system_message" => Ok(ContextEventKind::SystemMessage),
        "tool_call" => Ok(ContextEventKind::ToolCall),
        "tool_result" => Ok(ContextEventKind::ToolResult),
        "command_executed" => Ok(ContextEventKind::CommandExecuted),
        "file_read" => Ok(ContextEventKind::FileRead),
        "file_created" => Ok(ContextEventKind::FileCreated),
        "file_modified" => Ok(ContextEventKind::FileModified),
        "file_deleted" => Ok(ContextEventKind::FileDeleted),
        "file_moved" => Ok(ContextEventKind::FileMoved),
        "git_state" => Ok(ContextEventKind::GitState),
        "git_diff" => Ok(ContextEventKind::GitDiff),
        "test_run" => Ok(ContextEventKind::TestRun),
        "decision" => Ok(ContextEventKind::Decision),
        "assumption" => Ok(ContextEventKind::Assumption),
        "error" => Ok(ContextEventKind::Error),
        "checkpoint" => Ok(ContextEventKind::Checkpoint),
        "handoff" => Ok(ContextEventKind::Handoff),
        _ => Err(StorageError::Integrity(format!(
            "unknown event kind `{value}`"
        ))),
    }
}

pub(crate) const fn sensitivity_string(value: Sensitivity) -> &'static str {
    match value {
        Sensitivity::Normal => "normal",
        Sensitivity::PotentialSecret => "potential_secret",
        Sensitivity::Secret => "secret",
        Sensitivity::Excluded => "excluded",
    }
}

fn parse_sensitivity(value: &str) -> Result<Sensitivity, StorageError> {
    match value {
        "normal" => Ok(Sensitivity::Normal),
        "potential_secret" => Ok(Sensitivity::PotentialSecret),
        "secret" => Ok(Sensitivity::Secret),
        "excluded" => Ok(Sensitivity::Excluded),
        _ => Err(StorageError::Integrity(format!(
            "unknown sensitivity `{value}`"
        ))),
    }
}
