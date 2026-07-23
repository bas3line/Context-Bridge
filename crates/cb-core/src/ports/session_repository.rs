use async_trait::async_trait;

use crate::{
    BridgeSession, BridgeSessionId, ExternalSessionLink, ProjectId, ProjectRecord, SessionStatus,
};

#[async_trait]
pub trait SessionRepository: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    async fn upsert_project(&self, project: &ProjectRecord) -> Result<(), Self::Error>;
    async fn create_session(&self, session: &BridgeSession) -> Result<(), Self::Error>;
    async fn get_session(&self, id: BridgeSessionId) -> Result<Option<BridgeSession>, Self::Error>;
    async fn list_sessions(
        &self,
        project: Option<&ProjectId>,
    ) -> Result<Vec<BridgeSession>, Self::Error>;
    async fn last_session(
        &self,
        project: Option<&ProjectId>,
    ) -> Result<Option<BridgeSession>, Self::Error>;
    async fn update_session(
        &self,
        id: BridgeSessionId,
        status: SessionStatus,
        active_agent: Option<crate::AgentKind>,
    ) -> Result<(), Self::Error>;
    async fn link_external_session(&self, link: &ExternalSessionLink) -> Result<(), Self::Error>;
    async fn external_links(
        &self,
        session_id: BridgeSessionId,
    ) -> Result<Vec<ExternalSessionLink>, Self::Error>;
}
