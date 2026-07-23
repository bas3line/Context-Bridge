use crate::{AgentKind, BridgeSession, BridgeSessionId, Clock, ProjectId, SessionStatus};

#[derive(Debug, Default)]
pub struct SessionService;

impl SessionService {
    #[must_use]
    pub fn create(project_id: ProjectId, agent: AgentKind, clock: &dyn Clock) -> BridgeSession {
        let now = clock.now();
        BridgeSession {
            id: BridgeSessionId::new(),
            project_id,
            title: None,
            created_at: now,
            updated_at: now,
            status: SessionStatus::Active,
            active_agent: Some(agent),
        }
    }
}
