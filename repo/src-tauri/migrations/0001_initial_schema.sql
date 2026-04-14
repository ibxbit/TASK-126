-- Shoreline Property Operations Console — initial schema
-- Conventions:
--   id columns:    TEXT (UUIDv4/v7 as 36-char canonical form)
--   timestamps:    INTEGER Unix seconds, UTC
--   enums:         TEXT with CHECK constraint
--   encrypted:     BLOB — AES-256-GCM, 12-byte nonce prepended
--   multi-tenant:  every domain row carries tenant_id, enforced by FK
--                  and by application-layer permission guard.

PRAGMA foreign_keys = ON;
PRAGMA journal_mode = WAL;
PRAGMA synchronous  = NORMAL;

-- ──────────────────────────────────────────────────────────────────────
-- Tenants
-- ──────────────────────────────────────────────────────────────────────
CREATE TABLE tenants (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    code            TEXT NOT NULL UNIQUE,
    active          INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0,1)),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);
CREATE INDEX idx_tenants_active ON tenants(active);

-- ──────────────────────────────────────────────────────────────────────
-- Users & access control
-- ──────────────────────────────────────────────────────────────────────
CREATE TABLE users (
    id              TEXT PRIMARY KEY,
    username        TEXT NOT NULL UNIQUE,
    display_name    TEXT NOT NULL,
    password_hash   TEXT NOT NULL,               -- argon2id
    active          INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0,1)),
    last_login_at   INTEGER,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    created_by      TEXT REFERENCES users(id),
    updated_by      TEXT REFERENCES users(id)
);
CREATE INDEX idx_users_active ON users(active);

-- Static catalog of roles — mirrors the Rust `Role` enum. Kept in DB
-- for audit joins and for user-facing configuration screens.
CREATE TABLE roles (
    code            TEXT PRIMARY KEY
        CHECK (code IN ('administrator','property_manager','staff','reviewer','liaison')),
    label           TEXT NOT NULL,
    description     TEXT
);

CREATE TABLE permissions (
    code            TEXT PRIMARY KEY,
    label           TEXT NOT NULL
);

CREATE TABLE role_permissions (
    role_code       TEXT NOT NULL REFERENCES roles(code)       ON DELETE CASCADE,
    permission_code TEXT NOT NULL REFERENCES permissions(code) ON DELETE CASCADE,
    PRIMARY KEY (role_code, permission_code)
);

-- A user holds exactly one role; scope is either GLOBAL or a set of
-- tenant ids recorded in `user_role_tenants`.
CREATE TABLE user_roles (
    user_id         TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    role_code       TEXT NOT NULL    REFERENCES roles(code),
    scope_kind      TEXT NOT NULL CHECK (scope_kind IN ('global','tenants')),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    created_by      TEXT REFERENCES users(id),
    updated_by      TEXT REFERENCES users(id)
);

CREATE TABLE user_role_tenants (
    user_id         TEXT NOT NULL REFERENCES users(id)    ON DELETE CASCADE,
    tenant_id       TEXT NOT NULL REFERENCES tenants(id)  ON DELETE CASCADE,
    PRIMARY KEY (user_id, tenant_id)
);
CREATE INDEX idx_user_role_tenants_tenant ON user_role_tenants(tenant_id);

-- ──────────────────────────────────────────────────────────────────────
-- Residents (lightweight — only the fields needed by move-out flow)
-- ──────────────────────────────────────────────────────────────────────
CREATE TABLE residents (
    id                  TEXT PRIMARY KEY,
    tenant_id           TEXT NOT NULL REFERENCES tenants(id) ON DELETE RESTRICT,
    full_name           TEXT NOT NULL,
    email               TEXT,
    phone               TEXT,
    -- Encrypted blob: full national-id / SSN-like value
    national_id_enc     BLOB,
    -- Display-safe mask, e.g. "XXX-XX-1234"
    national_id_mask    TEXT,
    unit_label          TEXT,
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL,
    created_by          TEXT REFERENCES users(id),
    updated_by          TEXT REFERENCES users(id)
);
CREATE INDEX idx_residents_tenant       ON residents(tenant_id);
CREATE INDEX idx_residents_tenant_name  ON residents(tenant_id, full_name);

-- ──────────────────────────────────────────────────────────────────────
-- Move-Out Cases
-- ──────────────────────────────────────────────────────────────────────
CREATE TABLE move_out_cases (
    id                  TEXT PRIMARY KEY,
    tenant_id           TEXT NOT NULL REFERENCES tenants(id)   ON DELETE RESTRICT,
    resident_id         TEXT NOT NULL REFERENCES residents(id) ON DELETE RESTRICT,
    case_number         TEXT NOT NULL,
    status              TEXT NOT NULL CHECK (status IN
                            ('open','in_review','settled','closed','cancelled')),
    move_out_date       INTEGER NOT NULL,
    keys_returned_at    INTEGER,
    -- Encrypted blob: free-text case notes
    notes_enc           BLOB,
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL,
    created_by          TEXT REFERENCES users(id),
    updated_by          TEXT REFERENCES users(id),
    UNIQUE (tenant_id, case_number)
);
CREATE INDEX idx_cases_tenant_status    ON move_out_cases(tenant_id, status);
CREATE INDEX idx_cases_resident         ON move_out_cases(resident_id);
CREATE INDEX idx_cases_move_out_date    ON move_out_cases(tenant_id, move_out_date);

-- ──────────────────────────────────────────────────────────────────────
-- Deposits & Settlements
-- ──────────────────────────────────────────────────────────────────────
CREATE TABLE deposits (
    id              TEXT PRIMARY KEY,
    tenant_id       TEXT NOT NULL REFERENCES tenants(id) ON DELETE RESTRICT,
    case_id         TEXT NOT NULL REFERENCES move_out_cases(id) ON DELETE CASCADE,
    -- Integer minor units (cents) to avoid floating-point drift
    amount_cents    INTEGER NOT NULL CHECK (amount_cents >= 0),
    currency        TEXT NOT NULL DEFAULT 'USD',
    received_at     INTEGER NOT NULL,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    created_by      TEXT REFERENCES users(id),
    updated_by      TEXT REFERENCES users(id)
);
CREATE INDEX idx_deposits_case ON deposits(case_id);

CREATE TABLE settlements (
    id                  TEXT PRIMARY KEY,
    tenant_id           TEXT NOT NULL REFERENCES tenants(id) ON DELETE RESTRICT,
    case_id             TEXT NOT NULL REFERENCES move_out_cases(id) ON DELETE RESTRICT,
    deductions_cents    INTEGER NOT NULL DEFAULT 0 CHECK (deductions_cents >= 0),
    refund_cents        INTEGER NOT NULL DEFAULT 0 CHECK (refund_cents     >= 0),
    status              TEXT NOT NULL CHECK (status IN
                            ('draft','pending_approval','approved','paid','reopened','void')),
    approved_by         TEXT REFERENCES users(id),
    approved_at         INTEGER,
    paid_at             INTEGER,
    -- Encrypted blob: settlement notes (may reference PII)
    notes_enc           BLOB,
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL,
    created_by          TEXT REFERENCES users(id),
    updated_by          TEXT REFERENCES users(id)
);
CREATE INDEX idx_settlements_case           ON settlements(case_id);
CREATE INDEX idx_settlements_tenant_status  ON settlements(tenant_id, status);

-- ──────────────────────────────────────────────────────────────────────
-- Parcels
-- ──────────────────────────────────────────────────────────────────────
CREATE TABLE parcels (
    id              TEXT PRIMARY KEY,
    tenant_id       TEXT NOT NULL REFERENCES tenants(id)   ON DELETE RESTRICT,
    resident_id     TEXT NOT NULL REFERENCES residents(id) ON DELETE RESTRICT,
    tracking_code   TEXT,
    carrier         TEXT,
    status          TEXT NOT NULL CHECK (status IN
                        ('received','held','notified','delivered','returned')),
    received_at     INTEGER NOT NULL,
    delivered_at    INTEGER,
    delivered_to    TEXT,
    -- Encrypted blob: handoff / exception notes
    notes_enc       BLOB,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    created_by      TEXT REFERENCES users(id),
    updated_by      TEXT REFERENCES users(id)
);
CREATE INDEX idx_parcels_tenant_status   ON parcels(tenant_id, status);
CREATE INDEX idx_parcels_resident        ON parcels(resident_id);
CREATE INDEX idx_parcels_received_at     ON parcels(tenant_id, received_at);

-- ──────────────────────────────────────────────────────────────────────
-- Claims
-- ──────────────────────────────────────────────────────────────────────
CREATE TABLE claims (
    id                  TEXT PRIMARY KEY,
    tenant_id           TEXT NOT NULL REFERENCES tenants(id) ON DELETE RESTRICT,
    case_id             TEXT REFERENCES move_out_cases(id) ON DELETE SET NULL,
    claim_number        TEXT NOT NULL,
    category            TEXT NOT NULL CHECK (category IN
                            ('damage','cleaning','unpaid_rent','missing_item','other')),
    amount_cents        INTEGER NOT NULL CHECK (amount_cents >= 0),
    status              TEXT NOT NULL CHECK (status IN
                            ('new','in_review','accepted','rejected','reopened','closed')),
    opened_at           INTEGER NOT NULL,
    closed_at           INTEGER,
    -- Encrypted blob: claim narrative
    description_enc     BLOB,
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL,
    created_by          TEXT REFERENCES users(id),
    updated_by          TEXT REFERENCES users(id),
    UNIQUE (tenant_id, claim_number)
);
CREATE INDEX idx_claims_tenant_status   ON claims(tenant_id, status);
CREATE INDEX idx_claims_case            ON claims(case_id);
CREATE INDEX idx_claims_opened_at       ON claims(tenant_id, opened_at);

-- ──────────────────────────────────────────────────────────────────────
-- Attachments (metadata only — blobs live on disk)
-- ──────────────────────────────────────────────────────────────────────
CREATE TABLE attachments (
    id                      TEXT PRIMARY KEY,
    tenant_id               TEXT NOT NULL REFERENCES tenants(id) ON DELETE RESTRICT,
    entity_kind             TEXT NOT NULL CHECK (entity_kind IN
                                ('case','settlement','parcel','claim','resident')),
    entity_id               TEXT NOT NULL,
    -- Encrypted: original filename as provided by user
    display_name_enc        BLOB NOT NULL,
    -- Encrypted: relative path under attachments root
    relative_path_enc       BLOB NOT NULL,
    mime_type               TEXT NOT NULL,
    byte_size               INTEGER NOT NULL CHECK (byte_size >= 0),
    sha256_hex              TEXT NOT NULL,
    created_at              INTEGER NOT NULL,
    created_by              TEXT REFERENCES users(id),
    deleted_at              INTEGER,
    deleted_by              TEXT REFERENCES users(id)
);
CREATE INDEX idx_attachments_entity  ON attachments(entity_kind, entity_id);
CREATE INDEX idx_attachments_tenant  ON attachments(tenant_id);
CREATE INDEX idx_attachments_sha256  ON attachments(sha256_hex);

-- ──────────────────────────────────────────────────────────────────────
-- Events (analytics, append-only)
-- ──────────────────────────────────────────────────────────────────────
CREATE TABLE events (
    id              TEXT PRIMARY KEY,
    tenant_id       TEXT REFERENCES tenants(id),
    actor_user_id   TEXT REFERENCES users(id),
    session_id      TEXT,
    kind            TEXT NOT NULL,        -- e.g. 'case.opened', 'parcel.delivered'
    entity_kind     TEXT,
    entity_id       TEXT,
    -- Arbitrary JSON string describing the event. Must not contain
    -- sensitive PII — use `audit_log` for security-sensitive detail.
    payload_json    TEXT,
    occurred_at     INTEGER NOT NULL
);
CREATE INDEX idx_events_tenant_time ON events(tenant_id, occurred_at);
CREATE INDEX idx_events_entity      ON events(entity_kind, entity_id);
CREATE INDEX idx_events_kind_time   ON events(kind, occurred_at);

-- ──────────────────────────────────────────────────────────────────────
-- Audit log (security-sensitive, append-only, hash-chained)
-- ──────────────────────────────────────────────────────────────────────
CREATE TABLE audit_log (
    id              TEXT PRIMARY KEY,
    tenant_id       TEXT REFERENCES tenants(id),
    actor_user_id   TEXT REFERENCES users(id),
    session_id      TEXT,
    action          TEXT NOT NULL,
    entity_kind     TEXT,
    entity_id       TEXT,
    before_hash     TEXT,
    after_hash      TEXT,
    prev_chain_hash TEXT,
    chain_hash      TEXT NOT NULL,
    occurred_at     INTEGER NOT NULL
);
CREATE INDEX idx_audit_time     ON audit_log(occurred_at);
CREATE INDEX idx_audit_actor    ON audit_log(actor_user_id, occurred_at);

-- ──────────────────────────────────────────────────────────────────────
-- Scheduling rules (reminders, SLAs, recurring tasks)
-- ──────────────────────────────────────────────────────────────────────
CREATE TABLE scheduling_rules (
    id              TEXT PRIMARY KEY,
    tenant_id       TEXT NOT NULL REFERENCES tenants(id) ON DELETE RESTRICT,
    name            TEXT NOT NULL,
    trigger_kind    TEXT NOT NULL CHECK (trigger_kind IN
                        ('cron','relative','on_event')),
    -- For 'cron':      standard cron expression
    -- For 'relative':  ISO-8601 duration (e.g. "P3D", "PT2H")
    -- For 'on_event':  event kind string (e.g. "case.opened")
    trigger_spec    TEXT NOT NULL,
    action_kind     TEXT NOT NULL CHECK (action_kind IN
                        ('reminder','status_transition','escalate')),
    action_payload  TEXT NOT NULL,       -- JSON describing the action
    enabled         INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0,1)),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    created_by      TEXT REFERENCES users(id),
    updated_by      TEXT REFERENCES users(id)
);
CREATE INDEX idx_rules_tenant_enabled ON scheduling_rules(tenant_id, enabled);

-- ──────────────────────────────────────────────────────────────────────
-- Seed — role catalog
-- ──────────────────────────────────────────────────────────────────────
INSERT INTO roles(code, label, description) VALUES
    ('administrator',    'Administrator',          'Full configuration & user management'),
    ('property_manager', 'Property Manager',       'Approves settlements, reopens claims'),
    ('staff',            'Leasing / Front Desk',   'Parcel operations & resident submissions'),
    ('reviewer',         'Reviewer / Auditor',     'Read-only access with export'),
    ('liaison',          'Resident Liaison',       'Resident data entry & preferences');
