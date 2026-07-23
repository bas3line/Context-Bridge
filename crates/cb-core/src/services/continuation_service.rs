use crate::{AgentKind, BridgeSession, ExternalSessionLink};

#[derive(Debug, Default)]
pub struct ContinuationService;

impl ContinuationService {
    #[must_use]
    pub fn source_agent(
        session: &BridgeSession,
        links: &[ExternalSessionLink],
        requested: Option<AgentKind>,
    ) -> Option<AgentKind> {
        requested
            .or(session.active_agent)
            .or_else(|| links.last().map(|link| link.agent))
    }
}
