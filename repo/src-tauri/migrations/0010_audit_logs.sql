-- Centralized, append-only audit logging.
--
-- This table is intentionally separate from the pre-existing
-- `audit_log` (singular, hash-chained forensic trail). This one
-- captures structured business-level events with full before/after
-- JSON snapshots for replay and diff display.

PRAGMA foreign_keys = ON;

CREATE TABLE audit_logs (
    id              TEXT PRIMARY KEY,
    timestamp_unix  INTEGER NOT NULL,                          -- Unix seconds, UTC
    user_id         TEXT NOT NULL REFERENCES users(id),
    role            TEXT NOT NULL
        CHECK (role IN ('administrator','property_manager','staff','reviewer','liaison','system')),
    tenant_id       TEXT REFERENCES tenants(id),               -- nullable: global/system events
    action_type     TEXT NOT NULL,                             -- e.g. 'update', 'approve', 'delete'
    entity_type     TEXT NOT NULL,                             -- e.g. 'settlement', 'parcel'
    entity_id       TEXT,                                      -- nullable: bulk / collection actions
    before_state    TEXT,                                      -- JSON, nullable (create → null)
    after_state     TEXT,                                      -- JSON, nullable (delete → null)
    metadata        TEXT NOT NULL DEFAULT '{}'                 -- JSON; client / session context
);

-- Query indexes.
CREATE INDEX idx_audit_logs_timestamp     ON audit_logs(timestamp_unix);
CREATE INDEX idx_audit_logs_user_time     ON audit_logs(user_id, timestamp_unix);
CREATE INDEX idx_audit_logs_tenant_time   ON audit_logs(tenant_id, timestamp_unix);
CREATE INDEX idx_audit_logs_entity        ON audit_logs(entity_type, entity_id, timestamp_unix);
CREATE INDEX idx_audit_logs_action_time   ON audit_logs(action_type, timestamp_unix);

-- Append-only enforcement (defense in depth alongside the repository layer).
CREATE TRIGGER audit_logs_no_update
BEFORE UPDATE ON audit_logs
BEGIN
    SELECT RAISE(ABORT, 'audit_logs is append-only');
END;

CREATE TRIGGER audit_logs_no_delete
BEFORE DELETE ON audit_logs
BEGIN
    SELECT RAISE(ABORT, 'audit_logs is append-only');
END;
