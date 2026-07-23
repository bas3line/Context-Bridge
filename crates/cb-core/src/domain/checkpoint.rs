use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{BridgeSessionId, CheckpointId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkpoint {
    pub id: CheckpointId,
    pub bridge_session_id: BridgeSessionId,
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
    pub event_sequence: i64,
}
