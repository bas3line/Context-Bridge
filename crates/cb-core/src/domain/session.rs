use std::{fmt, path::PathBuf, str::FromStr};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::AgentKind;

macro_rules! uuid_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            #[must_use]
            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(value).map(Self)
            }
        }
    };
}

uuid_id!(BridgeSessionId);
uuid_id!(EventId);
uuid_id!(CheckpointId);
uuid_id!(HandoffId);

impl HandoffId {
    /// Derive a stable UUIDv7-shaped identifier from canonical handoff input.
    ///
    /// The timestamp portion is taken from the event-log watermark while the
    /// UUIDv7 random bits are deterministically filled from BLAKE3. This keeps
    /// rebuilt handoffs reproducible without weakening the UUID type boundary.
    #[must_use]
    pub fn from_deterministic_seed(timestamp: DateTime<Utc>, seed: &[u8]) -> Self {
        let milliseconds = timestamp.timestamp_millis().max(0) as u64;
        let mut bytes = [0_u8; 16];
        bytes[..6].copy_from_slice(&milliseconds.to_be_bytes()[2..]);
        let digest = blake3::hash(seed);
        bytes[6] = 0x70 | (digest.as_bytes()[0] & 0x0f);
        bytes[7] = digest.as_bytes()[1];
        bytes[8] = 0x80 | (digest.as_bytes()[2] & 0x3f);
        bytes[9..].copy_from_slice(&digest.as_bytes()[3..10]);
        Self(Uuid::from_bytes(bytes))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ExternalSessionId(String);

impl ExternalSessionId {
    pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(IdentifierError::Empty);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ExternalSessionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProjectId(String);

impl ProjectId {
    pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(IdentifierError::Empty);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn from_canonical_path(path: &std::path::Path) -> Self {
        Self(
            blake3::hash(path.to_string_lossy().as_bytes())
                .to_hex()
                .to_string(),
        )
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProjectId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum IdentifierError {
    #[error("identifier must not be empty")]
    Empty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Active,
    Paused,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeSession {
    pub id: BridgeSessionId,
    pub project_id: ProjectId,
    pub title: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub status: SessionStatus,
    pub active_agent: Option<AgentKind>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalSessionLink {
    pub bridge_session_id: BridgeSessionId,
    pub agent: AgentKind,
    pub external_session_id: ExternalSessionId,
    pub source_path: Option<PathBuf>,
    pub imported_at: DateTime<Utc>,
    pub last_synced_at: Option<DateTime<Utc>>,
    pub parser_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectRecord {
    pub id: ProjectId,
    pub root: PathBuf,
    pub is_git: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{ExternalSessionId, ProjectId};

    #[test]
    fn typed_identifiers_reject_empty_values() {
        assert!(ExternalSessionId::new("  ").is_err());
        assert!(ProjectId::new("").is_err());
    }

    #[test]
    fn project_identity_is_stable_for_a_canonical_path() {
        let first = ProjectId::from_canonical_path(Path::new("/tmp/context-bridge-project"));
        let second = ProjectId::from_canonical_path(Path::new("/tmp/context-bridge-project"));
        assert_eq!(first, second);
    }
}
