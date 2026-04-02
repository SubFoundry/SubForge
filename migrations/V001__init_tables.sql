CREATE TABLE IF NOT EXISTS plugins (
    id TEXT PRIMARY KEY,
    plugin_id TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    version TEXT NOT NULL,
    spec_version TEXT NOT NULL,
    type TEXT NOT NULL,
    status TEXT NOT NULL,
    installed_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS source_instances (
    id TEXT PRIMARY KEY,
    plugin_id TEXT NOT NULL,
    name TEXT NOT NULL,
    status TEXT NOT NULL,
    state_json TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS source_instance_config (
    id TEXT PRIMARY KEY,
    source_instance_id TEXT NOT NULL,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    FOREIGN KEY (source_instance_id) REFERENCES source_instances (id) ON DELETE CASCADE,
    UNIQUE (source_instance_id, key)
);

CREATE TABLE IF NOT EXISTS profiles (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS profile_sources (
    profile_id TEXT NOT NULL,
    source_instance_id TEXT NOT NULL,
    priority INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (profile_id, source_instance_id),
    FOREIGN KEY (profile_id) REFERENCES profiles (id) ON DELETE CASCADE,
    FOREIGN KEY (source_instance_id) REFERENCES source_instances (id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS refresh_jobs (
    id TEXT PRIMARY KEY,
    source_instance_id TEXT NOT NULL,
    trigger_type TEXT NOT NULL,
    status TEXT NOT NULL,
    started_at TEXT,
    finished_at TEXT,
    node_count INTEGER,
    error_code TEXT,
    error_message TEXT,
    FOREIGN KEY (source_instance_id) REFERENCES source_instances (id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS export_tokens (
    id TEXT PRIMARY KEY,
    profile_id TEXT NOT NULL,
    token TEXT NOT NULL UNIQUE,
    token_type TEXT NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT,
    FOREIGN KEY (profile_id) REFERENCES profiles (id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS app_settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS node_cache (
    id TEXT PRIMARY KEY,
    source_instance_id TEXT NOT NULL,
    data_json TEXT NOT NULL,
    fetched_at TEXT NOT NULL,
    expires_at TEXT,
    FOREIGN KEY (source_instance_id) REFERENCES source_instances (id) ON DELETE CASCADE
);
