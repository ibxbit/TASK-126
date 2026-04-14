-- Recovery + update tracking.
-- - app_versions: every install / rollback recorded as a row.
--   Exactly one row has `is_active = 1` at any time (partial unique
--   index); the previous row's `is_active = 0` row is the rollback
--   candidate.
-- - recovery_events: forensics — what the recovery manager observed
--   at each startup (clean / unclean / repaired).

PRAGMA foreign_keys = ON;

CREATE TABLE app_versions (
    id              TEXT PRIMARY KEY,
    version         TEXT NOT NULL,
    package_id      TEXT,                      -- id from the .spkg manifest
    installed_at    INTEGER NOT NULL,
    is_active       INTEGER NOT NULL DEFAULT 0 CHECK (is_active IN (0,1)),
    snapshot_path   TEXT,                      -- where /backups/<version>/ lives
    notes           TEXT,
    UNIQUE (version)
);
CREATE UNIQUE INDEX idx_app_versions_active_unique
    ON app_versions(is_active) WHERE is_active = 1;

CREATE TABLE recovery_events (
    id              TEXT PRIMARY KEY,
    started_at      INTEGER NOT NULL,
    completed_at    INTEGER NOT NULL,
    outcome         TEXT NOT NULL CHECK (outcome IN
                        ('clean_start','unclean_repaired','integrity_failed')),
    details         TEXT
);
CREATE INDEX idx_recovery_events_started ON recovery_events(started_at);
