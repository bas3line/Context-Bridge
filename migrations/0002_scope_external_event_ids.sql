-- Vendor event IDs are typically unique only within their external session.
-- Canonical import keys carry that session namespace, so this broader unique
-- index would incorrectly discard valid events from a second external session.
DROP INDEX IF EXISTS idx_context_events_external_event;

CREATE INDEX idx_context_events_external_event
    ON context_events(bridge_session_id, source_agent, external_event_id)
    WHERE external_event_id IS NOT NULL;

UPDATE schema_metadata
SET value = '2'
WHERE key = 'schema_version';
