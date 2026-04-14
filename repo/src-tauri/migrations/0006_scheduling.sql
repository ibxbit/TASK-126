-- Configurable scheduling engine: versioned rule sets, typed rules,
-- resources (staff / inspection slots), and assignments.

PRAGMA foreign_keys = ON;

-- ── Resources that can be scheduled ──────────────────────────────────
CREATE TABLE schedule_resources (
    id              TEXT PRIMARY KEY,
    tenant_id       TEXT NOT NULL REFERENCES tenants(id) ON DELETE RESTRICT,
    kind            TEXT NOT NULL CHECK (kind IN ('staff','inspection_slot')),
    name            TEXT NOT NULL,
    capacity        INTEGER NOT NULL DEFAULT 1 CHECK (capacity >= 1),
    user_id         TEXT REFERENCES users(id),       -- when kind='staff'
    enabled         INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0,1)),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    created_by      TEXT REFERENCES users(id),
    updated_by      TEXT REFERENCES users(id)
);
CREATE INDEX idx_schedule_resources_tenant_kind
    ON schedule_resources(tenant_id, kind, enabled);
CREATE INDEX idx_schedule_resources_user
    ON schedule_resources(user_id);

-- ── Versioned rule sets ──────────────────────────────────────────────
CREATE TABLE schedule_rule_sets (
    id              TEXT PRIMARY KEY,
    tenant_id       TEXT NOT NULL REFERENCES tenants(id) ON DELETE RESTRICT,
    name            TEXT NOT NULL,                   -- e.g. "Default Inspection Rules"
    version         INTEGER NOT NULL CHECK (version >= 1),
    parent_rule_set_id TEXT REFERENCES schedule_rule_sets(id),
    enabled         INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0,1)),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    created_by      TEXT REFERENCES users(id),
    updated_by      TEXT REFERENCES users(id),
    UNIQUE (tenant_id, name, version)
);

-- At most one enabled version per (tenant, name). Enforced via a
-- partial unique index on the boolean flag.
CREATE UNIQUE INDEX idx_schedule_rule_sets_active_unique
    ON schedule_rule_sets(tenant_id, name)
    WHERE enabled = 1;

CREATE INDEX idx_schedule_rule_sets_tenant_enabled
    ON schedule_rule_sets(tenant_id, enabled);

-- ── Individual rules within a rule set ───────────────────────────────
CREATE TABLE schedule_rules (
    id              TEXT PRIMARY KEY,
    rule_set_id     TEXT NOT NULL REFERENCES schedule_rule_sets(id) ON DELETE CASCADE,
    kind            TEXT NOT NULL CHECK (kind IN
                        ('unavailable_window','capacity_limit',
                         'required_duration','distribution')),
    severity        TEXT NOT NULL CHECK (severity IN ('hard','soft')),
    -- Typed JSON spec (parsed into a Rust enum at load time).
    spec_json       TEXT NOT NULL,
    -- Soft-rule weight (>0). Hard rules ignore this field.
    weight          INTEGER NOT NULL DEFAULT 1 CHECK (weight >= 0),
    enabled         INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0,1)),
    created_at      INTEGER NOT NULL
);
CREATE INDEX idx_schedule_rules_set ON schedule_rules(rule_set_id, enabled);

-- ── Existing schedule assignments (the "calendar") ───────────────────
CREATE TABLE schedule_assignments (
    id              TEXT PRIMARY KEY,
    tenant_id       TEXT NOT NULL REFERENCES tenants(id) ON DELETE RESTRICT,
    resource_id     TEXT NOT NULL REFERENCES schedule_resources(id) ON DELETE RESTRICT,
    subject_kind    TEXT NOT NULL CHECK (subject_kind IN
                        ('inspection','shift','case','custom')),
    subject_id      TEXT,
    start_unix      INTEGER NOT NULL,
    end_unix        INTEGER NOT NULL CHECK (end_unix > start_unix),
    notes_enc       BLOB,
    created_at      INTEGER NOT NULL,
    created_by      TEXT REFERENCES users(id),
    cancelled_at    INTEGER
);
CREATE INDEX idx_schedule_assign_resource_time
    ON schedule_assignments(resource_id, start_unix, end_unix);
CREATE INDEX idx_schedule_assign_tenant_time
    ON schedule_assignments(tenant_id, start_unix);
CREATE INDEX idx_schedule_assign_subject
    ON schedule_assignments(subject_kind, subject_id);
