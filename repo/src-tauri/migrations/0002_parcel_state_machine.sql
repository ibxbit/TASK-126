-- Parcel lifecycle state machine — states, configurable transition
-- rules, and the immutable per-parcel history log.
--
-- This migration redefines the `parcels.status` domain to match the
-- lifecycle spec (checked_in, checked_out, delivered, receipt_confirmed,
-- returned_exception). The rebuild is done via a rename+recreate+copy
-- pattern so the existing CHECK constraint can be replaced safely.

PRAGMA foreign_keys = OFF;

-- ── Rebuild parcels with the new status domain ───────────────────────
ALTER TABLE parcels RENAME TO parcels_legacy_0002;

CREATE TABLE parcels (
    id              TEXT PRIMARY KEY,
    tenant_id       TEXT NOT NULL REFERENCES tenants(id)   ON DELETE RESTRICT,
    resident_id     TEXT NOT NULL REFERENCES residents(id) ON DELETE RESTRICT,
    tracking_code   TEXT,
    carrier         TEXT,
    status          TEXT NOT NULL CHECK (status IN
                        ('checked_in','checked_out','delivered',
                         'receipt_confirmed','returned_exception')),
    received_at     INTEGER NOT NULL,
    delivered_at    INTEGER,
    delivered_to    TEXT,
    notes_enc       BLOB,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    created_by      TEXT REFERENCES users(id),
    updated_by      TEXT REFERENCES users(id)
);

-- Map legacy statuses onto the new domain. Anything not explicitly
-- mapped becomes 'checked_in' (safe default — never destroys history).
INSERT INTO parcels (
    id, tenant_id, resident_id, tracking_code, carrier, status,
    received_at, delivered_at, delivered_to, notes_enc,
    created_at, updated_at, created_by, updated_by
)
SELECT
    id, tenant_id, resident_id, tracking_code, carrier,
    CASE status
        WHEN 'received'  THEN 'checked_in'
        WHEN 'held'      THEN 'checked_in'
        WHEN 'notified'  THEN 'checked_in'
        WHEN 'delivered' THEN 'delivered'
        WHEN 'returned'  THEN 'returned_exception'
        ELSE 'checked_in'
    END,
    received_at, delivered_at, delivered_to, notes_enc,
    created_at, updated_at, created_by, updated_by
FROM parcels_legacy_0002;

DROP TABLE parcels_legacy_0002;

CREATE INDEX idx_parcels_tenant_status  ON parcels(tenant_id, status);
CREATE INDEX idx_parcels_resident       ON parcels(resident_id);
CREATE INDEX idx_parcels_received_at    ON parcels(tenant_id, received_at);

PRAGMA foreign_keys = ON;

-- ── Configurable transition rules ────────────────────────────────────
CREATE TABLE parcel_transition_rules (
    id              TEXT PRIMARY KEY,
    tenant_id       TEXT REFERENCES tenants(id) ON DELETE CASCADE,   -- NULL = global default
    from_state      TEXT NOT NULL,
    to_state        TEXT NOT NULL,
    guard_code      TEXT,            -- optional guard key (see `machine.rs`)
    required_permission TEXT,        -- e.g. 'parcel_operate'
    enabled         INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0,1)),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    created_by      TEXT REFERENCES users(id),
    updated_by      TEXT REFERENCES users(id),
    UNIQUE (tenant_id, from_state, to_state)
);
CREATE INDEX idx_parcel_rules_lookup
    ON parcel_transition_rules(tenant_id, from_state, enabled);

-- ── Immutable per-parcel history ─────────────────────────────────────
CREATE TABLE parcel_transitions (
    id                 TEXT PRIMARY KEY,
    tenant_id          TEXT NOT NULL REFERENCES tenants(id) ON DELETE RESTRICT,
    parcel_id          TEXT NOT NULL REFERENCES parcels(id) ON DELETE RESTRICT,
    from_state         TEXT,          -- NULL for the genesis (check-in) event
    to_state           TEXT NOT NULL,
    operator_user_id   TEXT NOT NULL REFERENCES users(id),
    occurred_at        INTEGER NOT NULL,
    location           TEXT NOT NULL, -- e.g. "Front Desk", "Locker Bay 2"
    notes_enc          BLOB,          -- optional encrypted free text
    prev_chain_hash    TEXT,
    chain_hash         TEXT NOT NULL  -- tamper-evidence over the whole record
);
CREATE INDEX idx_parcel_transitions_parcel ON parcel_transitions(parcel_id, occurred_at);
CREATE INDEX idx_parcel_transitions_tenant ON parcel_transitions(tenant_id, occurred_at);

-- Enforce immutability at the DB layer — defense in depth.
CREATE TRIGGER parcel_transitions_no_update
BEFORE UPDATE ON parcel_transitions
BEGIN
    SELECT RAISE(ABORT, 'parcel_transitions is append-only');
END;

CREATE TRIGGER parcel_transitions_no_delete
BEFORE DELETE ON parcel_transitions
BEGIN
    SELECT RAISE(ABORT, 'parcel_transitions is append-only');
END;

-- ── Seed: global default transition rules ────────────────────────────
-- (tenant_id = NULL → applies to every tenant unless overridden)
INSERT INTO parcel_transition_rules
    (id, tenant_id, from_state, to_state, guard_code, required_permission,
     enabled, created_at, updated_at)
VALUES
    -- Genesis: any parcel starts life checked in.
    ('00000000-0000-0000-0000-000000000001', NULL, '__genesis__', 'checked_in',
        NULL, 'parcel_operate', 1, strftime('%s','now'), strftime('%s','now')),

    -- Normal forward flow
    ('00000000-0000-0000-0000-000000000002', NULL, 'checked_in', 'checked_out',
        NULL, 'parcel_operate', 1, strftime('%s','now'), strftime('%s','now')),

    ('00000000-0000-0000-0000-000000000003', NULL, 'checked_out', 'delivered',
        'requires_check_in_exists', 'parcel_operate',
        1, strftime('%s','now'), strftime('%s','now')),

    ('00000000-0000-0000-0000-000000000004', NULL, 'delivered', 'receipt_confirmed',
        NULL, 'parcel_operate', 1, strftime('%s','now'), strftime('%s','now')),

    -- Exception paths — available from any active state
    ('00000000-0000-0000-0000-000000000005', NULL, 'checked_in', 'returned_exception',
        NULL, 'parcel_operate', 1, strftime('%s','now'), strftime('%s','now')),

    ('00000000-0000-0000-0000-000000000006', NULL, 'checked_out', 'returned_exception',
        NULL, 'parcel_operate', 1, strftime('%s','now'), strftime('%s','now')),

    ('00000000-0000-0000-0000-000000000007', NULL, 'delivered', 'returned_exception',
        NULL, 'parcel_operate', 1, strftime('%s','now'), strftime('%s','now'));
