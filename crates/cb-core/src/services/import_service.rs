use crate::{AgentKind, ExternalSessionId};

#[derive(Debug, Default)]
pub struct ImportService;

impl ImportService {
    #[must_use]
    pub fn import_namespace(
        agent: AgentKind,
        external_session_id: &ExternalSessionId,
        parser_version: &str,
    ) -> String {
        format!("{agent}:{external_session_id}:{parser_version}")
    }
}
