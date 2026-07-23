use std::str::FromStr;

use async_trait::async_trait;
use cb_core::{
    AgentKind, BridgeSession, BridgeSessionId, ExternalSessionId, ExternalSessionLink, ProjectId,
    ProjectRecord, SessionRepository, SessionStatus,
};
use sqlx::Row;

use crate::{SqliteStore, StorageError, sqlite::parse_datetime};

#[async_trait]
impl SessionRepository for SqliteStore {
    type Error = StorageError;

    async fn upsert_project(&self, project: &ProjectRecord) -> Result<(), Self::Error> {
        sqlx::query(
            "INSERT INTO projects (id, root, is_git, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
               root = excluded.root,
               is_git = excluded.is_git,
               updated_at = excluded.updated_at",
        )
        .bind(project.id.as_str())
        .bind(project.root.to_string_lossy().as_ref())
        .bind(i64::from(project.is_git))
        .bind(project.created_at.to_rfc3339())
        .bind(project.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(StorageError::Sqlx)?;
        Ok(())
    }

    async fn create_session(&self, session: &BridgeSession) -> Result<(), Self::Error> {
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
        .execute(&self.pool)
        .await
        .map_err(StorageError::Sqlx)?;
        Ok(())
    }

    async fn get_session(&self, id: BridgeSessionId) -> Result<Option<BridgeSession>, Self::Error> {
        let row = sqlx::query(
            "SELECT id, project_id, title, created_at, updated_at, status, active_agent
             FROM bridge_sessions WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(StorageError::Sqlx)?;
        row.map(row_to_session).transpose()
    }

    async fn list_sessions(
        &self,
        project: Option<&ProjectId>,
    ) -> Result<Vec<BridgeSession>, Self::Error> {
        let rows = if let Some(project) = project {
            sqlx::query(
                "SELECT id, project_id, title, created_at, updated_at, status, active_agent
                 FROM bridge_sessions WHERE project_id = ? ORDER BY updated_at DESC",
            )
            .bind(project.as_str())
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query(
                "SELECT id, project_id, title, created_at, updated_at, status, active_agent
                 FROM bridge_sessions ORDER BY updated_at DESC",
            )
            .fetch_all(&self.pool)
            .await
        }
        .map_err(StorageError::Sqlx)?;
        rows.into_iter().map(row_to_session).collect()
    }

    async fn last_session(
        &self,
        project: Option<&ProjectId>,
    ) -> Result<Option<BridgeSession>, Self::Error> {
        let row = if let Some(project) = project {
            sqlx::query(
                "SELECT id, project_id, title, created_at, updated_at, status, active_agent
                 FROM bridge_sessions WHERE project_id = ? ORDER BY updated_at DESC LIMIT 1",
            )
            .bind(project.as_str())
            .fetch_optional(&self.pool)
            .await
        } else {
            sqlx::query(
                "SELECT id, project_id, title, created_at, updated_at, status, active_agent
                 FROM bridge_sessions ORDER BY updated_at DESC LIMIT 1",
            )
            .fetch_optional(&self.pool)
            .await
        }
        .map_err(StorageError::Sqlx)?;
        row.map(row_to_session).transpose()
    }

    async fn update_session(
        &self,
        id: BridgeSessionId,
        status: SessionStatus,
        active_agent: Option<AgentKind>,
    ) -> Result<(), Self::Error> {
        sqlx::query(
            "UPDATE bridge_sessions
             SET status = ?, active_agent = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(status_string(status))
        .bind(active_agent.map(|agent| agent.to_string()))
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(id.to_string())
        .execute(&self.pool)
        .await
        .map_err(StorageError::Sqlx)?;
        Ok(())
    }

    async fn link_external_session(&self, link: &ExternalSessionLink) -> Result<(), Self::Error> {
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
        .execute(&self.pool)
        .await
        .map_err(StorageError::Sqlx)?;
        Ok(())
    }

    async fn external_links(
        &self,
        session_id: BridgeSessionId,
    ) -> Result<Vec<ExternalSessionLink>, Self::Error> {
        let rows = sqlx::query(
            "SELECT bridge_session_id, agent, external_session_id, source_path,
                    imported_at, last_synced_at, parser_version
             FROM external_session_links WHERE bridge_session_id = ?
             ORDER BY imported_at",
        )
        .bind(session_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(StorageError::Sqlx)?;
        rows.into_iter()
            .map(|row| {
                Ok(ExternalSessionLink {
                    bridge_session_id: row
                        .try_get::<String, _>("bridge_session_id")?
                        .parse::<BridgeSessionId>()?,
                    agent: AgentKind::from_str(&row.try_get::<String, _>("agent")?)
                        .map_err(|error| StorageError::Integrity(error.to_string()))?,
                    external_session_id: ExternalSessionId::new(
                        row.try_get::<String, _>("external_session_id")?,
                    )?,
                    source_path: row
                        .try_get::<Option<String>, _>("source_path")?
                        .map(Into::into),
                    imported_at: parse_datetime(&row.try_get::<String, _>("imported_at")?)?,
                    last_synced_at: row
                        .try_get::<Option<String>, _>("last_synced_at")?
                        .map(|value| parse_datetime(&value))
                        .transpose()?,
                    parser_version: row.try_get("parser_version")?,
                })
            })
            .collect()
    }
}

fn row_to_session(row: sqlx::sqlite::SqliteRow) -> Result<BridgeSession, StorageError> {
    Ok(BridgeSession {
        id: row.try_get::<String, _>("id")?.parse::<BridgeSessionId>()?,
        project_id: ProjectId::new(row.try_get::<String, _>("project_id")?)?,
        title: row.try_get("title")?,
        created_at: parse_datetime(&row.try_get::<String, _>("created_at")?)?,
        updated_at: parse_datetime(&row.try_get::<String, _>("updated_at")?)?,
        status: parse_status(&row.try_get::<String, _>("status")?)?,
        active_agent: row
            .try_get::<Option<String>, _>("active_agent")?
            .map(|value| {
                AgentKind::from_str(&value)
                    .map_err(|error| StorageError::Integrity(error.to_string()))
            })
            .transpose()?,
    })
}

pub(crate) const fn status_string(status: SessionStatus) -> &'static str {
    match status {
        SessionStatus::Active => "active",
        SessionStatus::Paused => "paused",
        SessionStatus::Completed => "completed",
        SessionStatus::Failed => "failed",
    }
}

fn parse_status(value: &str) -> Result<SessionStatus, StorageError> {
    match value {
        "active" => Ok(SessionStatus::Active),
        "paused" => Ok(SessionStatus::Paused),
        "completed" => Ok(SessionStatus::Completed),
        "failed" => Ok(SessionStatus::Failed),
        _ => Err(StorageError::Integrity(format!(
            "unknown session status `{value}`"
        ))),
    }
}
