use chrono::{DateTime, Utc};

use crate::{AgentKind, ContextEventKind, ContextEventPayload, NewContextEvent, Sensitivity};

#[derive(Debug, Default)]
pub struct CheckpointService;

impl CheckpointService {
    #[must_use]
    pub fn event(
        source_agent: Option<AgentKind>,
        timestamp: DateTime<Utc>,
        note: Option<String>,
        import_key: String,
        sensitivity: Sensitivity,
    ) -> NewContextEvent {
        NewContextEvent {
            source_agent,
            external_event_id: None,
            timestamp,
            kind: ContextEventKind::Checkpoint,
            payload: ContextEventPayload::Checkpoint {
                note,
                completed_work: Vec::new(),
                pending_tasks: Vec::new(),
                recommended_next_action: None,
            },
            sensitivity,
            import_metadata: None,
            parent_event_id: None,
            metadata: serde_json::Value::Null,
            import_key,
        }
    }
}
