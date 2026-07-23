use std::path::Path;

use cb_core::{
    AgentKind, ContextEventKind, ContextEventPayload, ImportMetadata, NewContextEvent,
    SecretScanner, Sensitivity,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawEvent {
    pub external_event_id: Option<String>,
    pub timestamp: Option<DateTime<Utc>>,
    pub kind: ContextEventKind,
    pub payload: ContextEventPayload,
    #[serde(default)]
    pub metadata: Value,
}

/// Stable provenance shared by all records from one external session import.
#[derive(Debug, Clone, Copy)]
pub struct NormalizationContext<'a> {
    pub agent: AgentKind,
    pub parser_name: &'a str,
    pub parser_version: &'a str,
    pub external_session_namespace: &'a str,
    pub source_path: Option<&'a Path>,
}

pub fn normalize_raw_event(
    raw: RawEvent,
    context: NormalizationContext<'_>,
    ordinal: usize,
    scanner: &dyn SecretScanner,
) -> NewContextEvent {
    let serialized = serde_json::to_string(&raw.payload).unwrap_or_default();
    let sensitivity = scanner.classify(&serialized);
    let external_key = raw
        .external_event_id
        .clone()
        .unwrap_or_else(|| format!("ordinal-{ordinal}"));
    NewContextEvent {
        source_agent: Some(context.agent),
        external_event_id: raw.external_event_id,
        timestamp: raw.timestamp.unwrap_or_else(Utc::now),
        kind: raw.kind,
        payload: raw.payload,
        sensitivity,
        import_metadata: Some(ImportMetadata {
            parser_name: context.parser_name.to_owned(),
            parser_version: context.parser_version.to_owned(),
            source_path: context.source_path.map(Path::to_path_buf),
            imported_at: Utc::now(),
        }),
        parent_event_id: None,
        metadata: raw.metadata,
        import_key: canonical_import_key(
            context.agent,
            context.external_session_namespace,
            context.parser_version,
            &external_key,
        ),
    }
}

/// Creates a stable import key scoped to one external agent session.
///
/// Vendor event identifiers are commonly only unique within an external
/// session. Length-prefixing makes the persisted key unambiguous even when a
/// vendor identifier contains a separator used by another component.
#[must_use]
pub fn canonical_import_key(
    agent: AgentKind,
    external_session_namespace: &str,
    parser_version: &str,
    external_event_key: &str,
) -> String {
    let agent = agent.to_string();
    format!(
        "event:{}:{}:{}:{}",
        length_prefixed(&agent),
        length_prefixed(external_session_namespace),
        length_prefixed(parser_version),
        length_prefixed(external_event_key),
    )
}

fn length_prefixed(value: &str) -> String {
    format!("{}:{value}", value.len())
}

#[must_use]
pub fn excluded_event(
    agent: AgentKind,
    kind: ContextEventKind,
    payload: ContextEventPayload,
    import_key: String,
) -> NewContextEvent {
    NewContextEvent {
        source_agent: Some(agent),
        external_event_id: None,
        timestamp: Utc::now(),
        kind,
        payload,
        sensitivity: Sensitivity::Excluded,
        import_metadata: None,
        parent_event_id: None,
        metadata: Value::Null,
        import_key,
    }
}

#[cfg(test)]
mod tests {
    use cb_core::AgentKind;

    use super::canonical_import_key;

    #[test]
    fn import_keys_scope_vendor_event_ids_to_the_external_session() {
        let first = canonical_import_key(AgentKind::ClaudeCode, "claude-session-a", "1", "1");
        let second = canonical_import_key(AgentKind::ClaudeCode, "claude-session-b", "1", "1");

        assert_ne!(first, second);
    }

    #[test]
    fn import_key_component_boundaries_are_unambiguous() {
        let first = canonical_import_key(AgentKind::Codex, "a:bc", "1", "event");
        let second = canonical_import_key(AgentKind::Codex, "a", "bc:1", "event");

        assert_ne!(first, second);
    }
}
