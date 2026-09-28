-- Schema version 1
CREATE TABLE IF NOT EXISTS devices (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    device_type TEXT NOT NULL,
    os TEXT NOT NULL,
    public_key TEXT NOT NULL,
    shared_secret TEXT NOT NULL,
    paired_at INTEGER NOT NULL,
    last_seen INTEGER NOT NULL,
    battery INTEGER,
    signal TEXT,
    status TEXT DEFAULT 'paired'
);

CREATE TABLE IF NOT EXISTS notifications (
    id TEXT PRIMARY KEY,
    device_id TEXT NOT NULL,
    app TEXT NOT NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    timestamp INTEGER NOT NULL,
    actions TEXT,
    dismissed INTEGER DEFAULT 0
);

CREATE TABLE IF NOT EXISTS clipboard_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    content TEXT NOT NULL,
    mime TEXT NOT NULL,
    source_device TEXT NOT NULL,
    timestamp INTEGER NOT NULL,
    pinned INTEGER DEFAULT 0
);

CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS file_transfers (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    size INTEGER NOT NULL,
    mime TEXT NOT NULL,
    from_device TEXT NOT NULL,
    to_device TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    chunks_received INTEGER NOT NULL DEFAULT 0,
    total_chunks INTEGER NOT NULL DEFAULT 0,
    saved_path TEXT,
    timestamp INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS automation_rules (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    trigger_type TEXT NOT NULL,
    trigger_config TEXT NOT NULL,
    action_type TEXT NOT NULL,
    action_config TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    last_triggered INTEGER
);

CREATE TABLE IF NOT EXISTS automation_logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    rule_id TEXT NOT NULL,
    trigger_type TEXT NOT NULL,
    timestamp INTEGER NOT NULL,
    success INTEGER NOT NULL,
    message TEXT
);
