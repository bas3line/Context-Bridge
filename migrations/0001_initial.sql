CREATE TABLE schema_metadata (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);

INSERT INTO schema_metadata (key, value) VALUES ('schema_version', '1');

CREATE TABLE projects (
    id TEXT PRIMARY KEY NOT NULL,
    root TEXT NOT NULL UNIQUE,
    is_git INTEGER NOT NULL CHECK (is_git IN (0, 1)),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE bridge_sessions (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE RESTRICT,
    title TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    status TEXT NOT NULL,
    active_agent TEXT
);

CREATE INDEX idx_bridge_sessions_project_updated
    ON bridge_sessions(project_id, updated_at DESC);

CREATE TABLE external_session_links (
    bridge_session_id TEXT NOT NULL REFERENCES bridge_sessions(id) ON DELETE CASCADE,
    agent TEXT NOT NULL,
    external_session_id TEXT NOT NULL,
    source_path TEXT,
    imported_at TEXT NOT NULL,
    last_synced_at TEXT,
    parser_version TEXT NOT NULL,
    PRIMARY KEY (bridge_session_id, agent, external_session_id),
    UNIQUE (agent, external_session_id)
);

CREATE TABLE context_events (
    id TEXT PRIMARY KEY NOT NULL,
    bridge_session_id TEXT NOT NULL REFERENCES bridge_sessions(id) ON DELETE CASCADE,
    source_agent TEXT,
    external_event_id TEXT,
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    timestamp TEXT NOT NULL,
    kind TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    sensitivity TEXT NOT NULL,
    import_metadata_json TEXT,
    parent_event_id TEXT REFERENCES context_events(id) ON DELETE SET NULL,
    metadata_json TEXT NOT NULL,
    import_key TEXT NOT NULL,
    UNIQUE (bridge_session_id, sequence),
    UNIQUE (bridge_session_id, import_key)
);

CREATE UNIQUE INDEX idx_context_events_external_event
    ON context_events(bridge_session_id, source_agent, external_event_id)
    WHERE external_event_id IS NOT NULL;

CREATE INDEX idx_context_events_session_sequence
    ON context_events(bridge_session_id, sequence);

CREATE TABLE checkpoints (
    id TEXT PRIMARY KEY NOT NULL,
    bridge_session_id TEXT NOT NULL REFERENCES bridge_sessions(id) ON DELETE CASCADE,
    note TEXT,
    created_at TEXT NOT NULL,
    event_sequence INTEGER NOT NULL
);

CREATE TABLE handoff_packages (
    id TEXT PRIMARY KEY NOT NULL,
    bridge_session_id TEXT NOT NULL REFERENCES bridge_sessions(id) ON DELETE CASCADE,
    source_agent TEXT NOT NULL,
    target_agent TEXT NOT NULL,
    schema_version INTEGER NOT NULL,
    package_json TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE artifacts (
    id TEXT PRIMARY KEY NOT NULL,
    bridge_session_id TEXT NOT NULL REFERENCES bridge_sessions(id) ON DELETE CASCADE,
    event_id TEXT REFERENCES context_events(id) ON DELETE SET NULL,
    path TEXT,
    content_hash TEXT NOT NULL,
    media_type TEXT,
    byte_len INTEGER NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE adapter_compatibility (
    agent TEXT NOT NULL,
    version_requirement TEXT NOT NULL,
    profile TEXT NOT NULL,
    parser_version TEXT NOT NULL,
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    PRIMARY KEY (agent, version_requirement, profile)
);
