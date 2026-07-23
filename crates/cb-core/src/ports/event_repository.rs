use async_trait::async_trait;

use crate::{BridgeSessionId, ContextEvent, NewContextEvent};

#[async_trait]
pub trait EventRepository: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    async fn append_events(
        &self,
        session_id: BridgeSessionId,
        events: &[NewContextEvent],
    ) -> Result<Vec<ContextEvent>, Self::Error>;
    async fn events(&self, session_id: BridgeSessionId) -> Result<Vec<ContextEvent>, Self::Error>;
    async fn event_count(&self, session_id: BridgeSessionId) -> Result<u64, Self::Error>;
}
