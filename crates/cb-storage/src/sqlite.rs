use std::{io::ErrorKind, path::Path, str::FromStr};

use cb_core::{
    AgentKind, BridgeSessionId, Checkpoint, CheckpointId, HandoffPackage, ProjectId, ProjectRecord,
};
use chrono::{DateTime, Utc};
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};

use crate::migrations;

#[derive(Debug, Clone)]
pub struct SqliteStore {
    pub(crate) pool: SqlitePool,
}

impl SqliteStore {
    pub async fn open(path: &Path) -> Result<Self, StorageError> {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        ensure_private_data_directory(parent)?;

        let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))
            .map_err(StorageError::Configuration)?
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Full)
            .busy_timeout(std::time::Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await
            .map_err(StorageError::Sqlx)?;
        migrations::run(&pool).await?;
        set_restrictive_permissions(path, false)?;
        for suffix in ["-wal", "-shm"] {
            let sidecar = std::path::PathBuf::from(format!("{}{suffix}", path.display()));
            if sidecar.exists() {
                set_restrictive_permissions(&sidecar, false)?;
            }
        }
        Ok(Self { pool })
    }

    pub async fn health_check(&self) -> Result<(), StorageError> {
        let result: String = sqlx::query_scalar("PRAGMA quick_check")
            .fetch_one(&self.pool)
            .await
            .map_err(StorageError::Sqlx)?;
        if result == "ok" {
            Ok(())
        } else {
            Err(StorageError::Integrity(format!(
                "SQLite quick_check returned {result}"
            )))
        }
    }

    pub async fn project(&self, id: &ProjectId) -> Result<Option<ProjectRecord>, StorageError> {
        let row = sqlx::query(
            "SELECT id, root, is_git, created_at, updated_at FROM projects WHERE id = ?",
        )
        .bind(id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(StorageError::Sqlx)?;
        row.map(|row| {
            Ok(ProjectRecord {
                id: ProjectId::new(row.try_get::<String, _>("id")?)?,
                root: row.try_get::<String, _>("root")?.into(),
                is_git: row.try_get::<i64, _>("is_git")? != 0,
                created_at: parse_datetime(&row.try_get::<String, _>("created_at")?)?,
                updated_at: parse_datetime(&row.try_get::<String, _>("updated_at")?)?,
            })
        })
        .transpose()
    }

    pub async fn create_checkpoint(&self, checkpoint: &Checkpoint) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO checkpoints
             (id, bridge_session_id, note, created_at, event_sequence)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(checkpoint.id.to_string())
        .bind(checkpoint.bridge_session_id.to_string())
        .bind(&checkpoint.note)
        .bind(checkpoint.created_at.to_rfc3339())
        .bind(checkpoint.event_sequence)
        .execute(&self.pool)
        .await
        .map_err(StorageError::Sqlx)?;
        Ok(())
    }

    pub async fn checkpoints(
        &self,
        session_id: BridgeSessionId,
    ) -> Result<Vec<Checkpoint>, StorageError> {
        let rows = sqlx::query(
            "SELECT id, bridge_session_id, note, created_at, event_sequence
             FROM checkpoints WHERE bridge_session_id = ? ORDER BY event_sequence",
        )
        .bind(session_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(StorageError::Sqlx)?;
        rows.into_iter()
            .map(|row| {
                Ok(Checkpoint {
                    id: row.try_get::<String, _>("id")?.parse::<CheckpointId>()?,
                    bridge_session_id: row
                        .try_get::<String, _>("bridge_session_id")?
                        .parse::<BridgeSessionId>()?,
                    note: row.try_get("note")?,
                    created_at: parse_datetime(&row.try_get::<String, _>("created_at")?)?,
                    event_sequence: row.try_get("event_sequence")?,
                })
            })
            .collect()
    }

    pub async fn save_handoff(&self, package: &HandoffPackage) -> Result<(), StorageError> {
        let package_json = serde_json::to_string(package)?;
        let content_hash = blake3::hash(package_json.as_bytes()).to_hex().to_string();
        sqlx::query(
            "INSERT INTO handoff_packages
             (id, bridge_session_id, source_agent, target_agent, schema_version,
              package_json, content_hash, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
               bridge_session_id = excluded.bridge_session_id,
               source_agent = excluded.source_agent,
               target_agent = excluded.target_agent,
               schema_version = excluded.schema_version,
               package_json = excluded.package_json,
               content_hash = excluded.content_hash,
               created_at = excluded.created_at",
        )
        .bind(package.id.to_string())
        .bind(package.session_id.to_string())
        .bind(package.source_agent.to_string())
        .bind(package.target_agent.to_string())
        .bind(i64::from(package.schema_version))
        .bind(package_json)
        .bind(content_hash)
        .bind(package.generated_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(StorageError::Sqlx)?;
        Ok(())
    }

    pub async fn latest_handoff(
        &self,
        session_id: BridgeSessionId,
    ) -> Result<Option<HandoffPackage>, StorageError> {
        let json: Option<String> = sqlx::query_scalar(
            "SELECT package_json FROM handoff_packages
             WHERE bridge_session_id = ? ORDER BY created_at DESC, rowid DESC LIMIT 1",
        )
        .bind(session_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(StorageError::Sqlx)?;
        json.map(|value| serde_json::from_str(&value).map_err(StorageError::Json))
            .transpose()
    }

    pub async fn schema_version(&self) -> Result<String, StorageError> {
        sqlx::query_scalar("SELECT value FROM schema_metadata WHERE key = 'schema_version'")
            .fetch_one(&self.pool)
            .await
            .map_err(StorageError::Sqlx)
    }

    pub async fn adapter_link_exists(
        &self,
        agent: AgentKind,
        external_id: &str,
    ) -> Result<bool, StorageError> {
        let exists: i64 = sqlx::query_scalar(
            "SELECT EXISTS(
                SELECT 1 FROM external_session_links
                WHERE agent = ? AND external_session_id = ?
             )",
        )
        .bind(agent.to_string())
        .bind(external_id)
        .fetch_one(&self.pool)
        .await
        .map_err(StorageError::Sqlx)?;
        Ok(exists != 0)
    }

    pub async fn session_for_external(
        &self,
        agent: AgentKind,
        external_id: &str,
    ) -> Result<Option<BridgeSessionId>, StorageError> {
        let id: Option<String> = sqlx::query_scalar(
            "SELECT bridge_session_id FROM external_session_links
             WHERE agent = ? AND external_session_id = ?",
        )
        .bind(agent.to_string())
        .bind(external_id)
        .fetch_optional(&self.pool)
        .await?;
        id.map(|value| value.parse::<BridgeSessionId>().map_err(StorageError::Uuid))
            .transpose()
    }
}

pub(crate) fn parse_datetime(value: &str) -> Result<DateTime<Utc>, StorageError> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(StorageError::Chrono)
}

fn ensure_private_data_directory(path: &Path) -> Result<(), StorageError> {
    let mut missing = Vec::new();
    let mut cursor = path;
    loop {
        match std::fs::symlink_metadata(cursor) {
            Ok(metadata) => {
                if !metadata.file_type().is_dir() {
                    return Err(StorageError::InvalidDataDirectory {
                        path: cursor.to_path_buf(),
                    });
                }
                break;
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                missing.push(cursor.to_path_buf());
                cursor = cursor
                    .parent()
                    .ok_or_else(|| StorageError::InvalidDataDirectory {
                        path: path.to_path_buf(),
                    })?;
            }
            Err(source) => {
                return Err(StorageError::InspectDataDirectory {
                    path: cursor.to_path_buf(),
                    source,
                });
            }
        }
    }
    for directory in missing.iter().rev() {
        match create_private_data_directory(directory) {
            Ok(()) => set_restrictive_permissions(directory, true)?,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                validate_private_data_directory(directory)?;
            }
            Err(source) => {
                return Err(StorageError::CreateDataDirectory {
                    path: directory.clone(),
                    source,
                });
            }
        }
    }
    validate_private_data_directory(path)
}

#[cfg(unix)]
fn create_private_data_directory(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    let mut builder = std::fs::DirBuilder::new();
    builder.mode(0o700).create(path)
}

#[cfg(not(unix))]
fn create_private_data_directory(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir(path)
}

fn validate_private_data_directory(path: &Path) -> Result<(), StorageError> {
    let metadata =
        std::fs::symlink_metadata(path).map_err(|source| StorageError::InspectDataDirectory {
            path: path.to_path_buf(),
            source,
        })?;
    if !metadata.file_type().is_dir() {
        return Err(StorageError::InvalidDataDirectory {
            path: path.to_path_buf(),
        });
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(StorageError::InsecureDataDirectory {
                path: path.to_path_buf(),
            });
        }
    }
    Ok(())
}

#[cfg(unix)]
fn set_restrictive_permissions(path: &Path, directory: bool) -> Result<(), StorageError> {
    use std::os::unix::fs::PermissionsExt;
    let mode = if directory { 0o700 } else { 0o600 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).map_err(|source| {
        StorageError::SetPermissions {
            path: path.to_path_buf(),
            source,
        }
    })
}

#[cfg(not(unix))]
fn set_restrictive_permissions(_path: &Path, _directory: bool) -> Result<(), StorageError> {
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("could not create data directory `{path}`")]
    CreateDataDirectory {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not inspect data directory `{path}`")]
    InspectDataDirectory {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("data directory `{path}` must be a real directory, not a file or symlink")]
    InvalidDataDirectory { path: std::path::PathBuf },
    #[error(
        "refusing to use existing data directory `{path}` because it is accessible by group or other users"
    )]
    InsecureDataDirectory { path: std::path::PathBuf },
    #[error("could not set restrictive permissions on `{path}`")]
    SetPermissions {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid SQLite configuration")]
    Configuration(#[source] sqlx::Error),
    #[error("SQLite operation failed")]
    Sqlx(#[from] sqlx::Error),
    #[error("database migration failed")]
    Migration(#[source] sqlx::migrate::MigrateError),
    #[error("database integrity check failed: {0}")]
    Integrity(String),
    #[error("stored timestamp is invalid")]
    Chrono(#[source] chrono::ParseError),
    #[error("stored identifier is invalid")]
    Uuid(#[from] uuid::Error),
    #[error("stored typed identifier is invalid")]
    Identifier(#[from] cb_core::IdentifierError),
    #[error("stored JSON is invalid")]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use cb_core::{
        AgentKind, BridgeSession, BridgeSessionId, ContextEventKind, ContextEventPayload,
        EventRepository, ExternalSessionId, ExternalSessionLink, GitContext, HandoffId,
        HandoffPackage, NewContextEvent, ProjectId, ProjectRecord, ProjectSummary, Sensitivity,
        SessionRepository, SessionStatus,
    };
    use chrono::Utc;

    use super::{SqliteStore, StorageError};

    #[tokio::test]
    async fn append_is_ordered_idempotent_and_rollback_safe() {
        let directory = tempfile::tempdir().expect("temporary storage directory");
        let path = directory.path().join("data").join("bridge.db");
        let store = SqliteStore::open(&path).await.expect("open store");
        let unknown = BridgeSessionId::new();
        assert!(
            store
                .append_events(unknown, &[event("missing", "missing")])
                .await
                .is_err(),
            "foreign-key failure must abort the import transaction"
        );
        store
            .health_check()
            .await
            .expect("database remains healthy");

        let project_id = ProjectId::new("project").expect("project id");
        let now = Utc::now();
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
        let handoff = handoff(session_id, directory.path().to_path_buf(), now);
        store
            .save_handoff(&handoff)
            .await
            .expect("first handoff save");
        store
            .save_handoff(&handoff)
            .await
            .expect("repeat handoff save is idempotent");
        assert_eq!(
            store
                .latest_handoff(session_id)
                .await
                .expect("latest handoff"),
            Some(handoff)
        );
        let first = event("event-1", "first");
        assert_eq!(
            store
                .append_events(session_id, std::slice::from_ref(&first))
                .await
                .expect("first import")
                .len(),
            1
        );
        assert!(
            store
                .append_events(session_id, &[first])
                .await
                .expect("duplicate import")
                .is_empty()
        );
        store
            .append_events(session_id, &[event("event-2", "second")])
            .await
            .expect("second event");
        let events = store.events(session_id).await.expect("events");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].sequence, 1);
        assert_eq!(events[1].sequence, 2);

        let namespaced_events = [
            event_with_external_id(
                "event:claude-session-a:shared-id",
                "shared-id",
                "first external session",
            ),
            event_with_external_id(
                "event:claude-session-b:shared-id",
                "shared-id",
                "second external session",
            ),
        ];
        assert_eq!(
            store
                .append_events(session_id, &namespaced_events)
                .await
                .expect("same vendor event ID from separate external sessions")
                .len(),
            2
        );
        assert_eq!(store.event_count(session_id).await.expect("event count"), 4);
        store.health_check().await.expect("healthy after imports");

        let external_id = ExternalSessionId::new("duplicate-external").expect("external id");
        store
            .link_external_session(&ExternalSessionLink {
                bridge_session_id: session_id,
                agent: AgentKind::ClaudeCode,
                external_session_id: external_id.clone(),
                source_path: None,
                imported_at: now,
                last_synced_at: Some(now),
                parser_version: "1".to_owned(),
            })
            .await
            .expect("first external link");
        let conflicting_id = BridgeSessionId::new();
        let conflicting_session = BridgeSession {
            id: conflicting_id,
            project_id: ProjectId::new("project").expect("project id"),
            title: None,
            created_at: now,
            updated_at: now,
            status: SessionStatus::Paused,
            active_agent: Some(AgentKind::ClaudeCode),
        };
        let conflict = store
            .import_session_bundle(
                Some(&conflicting_session),
                conflicting_id,
                &[event("conflicting-event", "must roll back")],
                &ExternalSessionLink {
                    bridge_session_id: conflicting_id,
                    agent: AgentKind::ClaudeCode,
                    external_session_id: external_id,
                    source_path: None,
                    imported_at: now,
                    last_synced_at: Some(now),
                    parser_version: "1".to_owned(),
                },
            )
            .await;
        assert!(conflict.is_err());
        assert!(
            store
                .get_session(conflicting_id)
                .await
                .expect("query rolled-back session")
                .is_none()
        );
        assert_eq!(
            store
                .event_count(conflicting_id)
                .await
                .expect("rolled-back event count"),
            0
        );
        store.health_check().await.expect("healthy after rollback");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for candidate in [
                path.clone(),
                format!("{}-wal", path.display()).into(),
                format!("{}-shm", path.display()).into(),
            ] {
                if candidate.exists() {
                    assert_eq!(
                        candidate.metadata().expect("metadata").permissions().mode() & 0o077,
                        0
                    );
                }
            }
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn refuses_public_existing_data_directory_without_chmodding_it() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().expect("temporary root");
        let directory = root.path().join("shared-directory");
        std::fs::create_dir(&directory).expect("create shared directory");
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755))
            .expect("make directory public");
        let before = directory
            .metadata()
            .expect("metadata before open")
            .permissions()
            .mode()
            & 0o777;

        let error = SqliteStore::open(&directory.join("bridge.db"))
            .await
            .expect_err("public existing directory must be rejected");
        assert!(matches!(error, StorageError::InsecureDataDirectory { .. }));
        let after = directory
            .metadata()
            .expect("metadata after open")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            after, before,
            "opening storage must not chmod an existing parent"
        );
        assert!(!directory.join("bridge.db").exists());
    }

    fn event(import_key: &str, content: &str) -> NewContextEvent {
        event_with_external_id(import_key, import_key, content)
    }

    fn event_with_external_id(
        import_key: &str,
        external_event_id: &str,
        content: &str,
    ) -> NewContextEvent {
        NewContextEvent {
            source_agent: Some(AgentKind::ClaudeCode),
            external_event_id: Some(external_event_id.to_owned()),
            timestamp: Utc::now(),
            kind: ContextEventKind::UserMessage,
            payload: ContextEventPayload::Message {
                content: content.to_owned(),
            },
            sensitivity: Sensitivity::Normal,
            import_metadata: None,
            parent_event_id: None,
            metadata: serde_json::Value::Null,
            import_key: import_key.to_owned(),
        }
    }

    fn handoff(
        session_id: BridgeSessionId,
        root: std::path::PathBuf,
        generated_at: chrono::DateTime<Utc>,
    ) -> HandoffPackage {
        HandoffPackage {
            id: HandoffId::new(),
            schema_version: 1,
            session_id,
            source_agent: AgentKind::ClaudeCode,
            target_agent: AgentKind::Codex,
            project: ProjectSummary {
                id: "project".to_owned(),
                root,
            },
            original_objective: None,
            current_objective: None,
            completed_work: Vec::new(),
            current_state: Vec::new(),
            decisions: Vec::new(),
            assumptions: Vec::new(),
            failed_approaches: Vec::new(),
            modified_files: Vec::new(),
            relevant_files: Vec::new(),
            commands: Vec::new(),
            tests: Vec::new(),
            errors: Vec::new(),
            pending_tasks: Vec::new(),
            recommended_next_action: None,
            recent_conversation: Vec::new(),
            git: GitContext::default(),
            generated_at,
        }
    }
}
