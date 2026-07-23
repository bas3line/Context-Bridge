use cb_core::{BridgeSession, BridgeSessionId, ExternalSessionLink, NewContextEvent};

use crate::{
    SqliteStore, StorageError,
    event_repository::{event_kind_string, sensitivity_string},
    session_repository::status_string,
};

impl SqliteStore {
    pub async fn import_session_bundle(
        &self,
        new_session: Option<&BridgeSession>,
        session_id: BridgeSessionId,
        events: &[NewContextEvent],
        link: &ExternalSessionLink,
    ) -> Result<usize, StorageError> {
        // This bundle also allocates event sequences via MAX(sequence), so it must acquire the
        // SQLite writer reservation before that read. It shares the same serialization boundary
        // as ordinary event appends.
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        if let Some(session) = new_session {
            sqlx::query(
                "INSERT INTO bridge_sessions
                 (id, project_id, title, created_at, updated_at, status, active_agent)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(session.id.to_string())
            .bind(session.project_id.as_str())
            .bind(&session.title)
            .bind(session.created_at.to_rfc3339())
            .bind(session.updated_at.to_rfc3339())
            .bind(status_string(session.status))
            .bind(session.active_agent.map(|agent| agent.to_string()))
            .execute(&mut *transaction)
            .await?;
        }

        let mut sequence: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(sequence), 0)
             FROM context_events WHERE bridge_session_id = ?",
        )
        .bind(session_id.to_string())
        .fetch_one(&mut *transaction)
        .await?;
        let mut inserted = 0_usize;
        for event in events {
            sequence += 1;
            let result = sqlx::query(
                "INSERT OR IGNORE INTO context_events
                 (id, bridge_session_id, source_agent, external_event_id, sequence,
                  timestamp, kind, payload_json, content_hash, sensitivity,
                  import_metadata_json, parent_event_id, metadata_json, import_key)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(cb_core::EventId::new().to_string())
            .bind(session_id.to_string())
            .bind(event.source_agent.map(|agent| agent.to_string()))
            .bind(&event.external_event_id)
            .bind(sequence)
            .bind(event.timestamp.to_rfc3339())
            .bind(event_kind_string(event.kind))
            .bind(serde_json::to_string(&event.payload)?)
            .bind(event.content_hash()?)
            .bind(sensitivity_string(event.sensitivity))
            .bind(
                event
                    .import_metadata
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()?,
            )
            .bind(event.parent_event_id.map(|parent| parent.to_string()))
            .bind(serde_json::to_string(&event.metadata)?)
            .bind(&event.import_key)
            .execute(&mut *transaction)
            .await?;
            if result.rows_affected() == 0 {
                sequence -= 1;
            } else {
                inserted += 1;
            }
        }

        sqlx::query(
            "INSERT INTO external_session_links
             (bridge_session_id, agent, external_session_id, source_path, imported_at,
              last_synced_at, parser_version)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(bridge_session_id, agent, external_session_id) DO UPDATE SET
               source_path = COALESCE(excluded.source_path, source_path),
               last_synced_at = excluded.last_synced_at,
               parser_version = excluded.parser_version",
        )
        .bind(link.bridge_session_id.to_string())
        .bind(link.agent.to_string())
        .bind(link.external_session_id.as_str())
        .bind(
            link.source_path
                .as_ref()
                .map(|path| path.to_string_lossy().to_string()),
        )
        .bind(link.imported_at.to_rfc3339())
        .bind(link.last_synced_at.map(|time| time.to_rfc3339()))
        .bind(&link.parser_version)
        .execute(&mut *transaction)
        .await?;
        sqlx::query("UPDATE bridge_sessions SET updated_at = ? WHERE id = ?")
            .bind(chrono::Utc::now().to_rfc3339())
            .bind(session_id.to_string())
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(inserted)
    }
}
