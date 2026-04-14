-- Settlement workflow: deposit hold/release, inspections,
-- deduction line items, two-step approval, check requests, ledger.

PRAGMA foreign_keys = ON;

-- ── Extend deposits with hold/release state ──────────────────────────
ALTER TABLE deposits ADD COLUMN hold_status TEXT NOT NULL DEFAULT 'held'
    CHECK (hold_status IN ('held','released','forfeited_partial','forfeited_full'));
ALTER TABLE deposits ADD COLUMN held_at      INTEGER;
ALTER TABLE deposits ADD COLUMN released_at  INTEGER;
CREATE INDEX idx_deposits_hold_status ON deposits(hold_status);

-- ── Inspections (move-in / pre-move-out / final) ─────────────────────
CREATE TABLE inspections (
    id              TEXT PRIMARY KEY,
    tenant_id       TEXT NOT NULL REFERENCES tenants(id) ON DELETE RESTRICT,
    case_id         TEXT NOT NULL REFERENCES move_out_cases(id) ON DELETE CASCADE,
    kind            TEXT NOT NULL CHECK (kind IN
                        ('move_in','pre_move_out','final')),
    inspector_user_id TEXT NOT NULL REFERENCES users(id),
    performed_at    INTEGER NOT NULL,
    -- Encrypted free text — observations / damage notes.
    notes_enc       BLOB,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    created_by      TEXT REFERENCES users(id),
    updated_by      TEXT REFERENCES users(id)
);
CREATE INDEX idx_inspections_case ON inspections(case_id, kind);

-- ── Deduction line items per settlement ──────────────────────────────
CREATE TABLE deduction_items (
    id              TEXT PRIMARY KEY,
    tenant_id       TEXT NOT NULL REFERENCES tenants(id) ON DELETE RESTRICT,
    settlement_id   TEXT NOT NULL REFERENCES settlements(id) ON DELETE CASCADE,
    category        TEXT NOT NULL CHECK (category IN
                        ('damage','cleaning','unpaid_rent','missing_item',
                         'utilities','other')),
    description     TEXT NOT NULL,
    amount_cents    INTEGER NOT NULL CHECK (amount_cents >= 0),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    created_by      TEXT REFERENCES users(id),
    updated_by      TEXT REFERENCES users(id)
);
CREATE INDEX idx_deduction_items_settlement ON deduction_items(settlement_id);

-- ── Evidence: M:N from deduction_items to attachments ────────────────
CREATE TABLE deduction_evidence (
    deduction_item_id   TEXT NOT NULL REFERENCES deduction_items(id) ON DELETE CASCADE,
    attachment_id       TEXT NOT NULL REFERENCES attachments(id)     ON DELETE RESTRICT,
    created_at          INTEGER NOT NULL,
    PRIMARY KEY (deduction_item_id, attachment_id)
);
CREATE INDEX idx_deduction_evidence_attachment ON deduction_evidence(attachment_id);

-- ── Two-step approval signatures (immutable) ─────────────────────────
CREATE TABLE settlement_approvals (
    id              TEXT PRIMARY KEY,
    settlement_id   TEXT NOT NULL REFERENCES settlements(id) ON DELETE CASCADE,
    step            TEXT NOT NULL CHECK (step IN ('prepared','approved')),
    user_id         TEXT NOT NULL REFERENCES users(id),
    signed_at       INTEGER NOT NULL,
    notes_enc       BLOB,
    UNIQUE (settlement_id, step)
);
CREATE INDEX idx_settlement_approvals_settlement ON settlement_approvals(settlement_id);

CREATE TRIGGER settlement_approvals_no_update
BEFORE UPDATE ON settlement_approvals
BEGIN
    SELECT RAISE(ABORT, 'settlement_approvals is append-only');
END;

CREATE TRIGGER settlement_approvals_no_delete
BEFORE DELETE ON settlement_approvals
BEGIN
    SELECT RAISE(ABORT, 'settlement_approvals is append-only');
END;

-- ── Printable check requests (offline only) ──────────────────────────
CREATE TABLE check_requests (
    id                  TEXT PRIMARY KEY,
    tenant_id           TEXT NOT NULL REFERENCES tenants(id) ON DELETE RESTRICT,
    settlement_id       TEXT NOT NULL REFERENCES settlements(id) ON DELETE RESTRICT,
    payee_name          TEXT NOT NULL,
    amount_cents        INTEGER NOT NULL CHECK (amount_cents > 0),
    currency            TEXT NOT NULL DEFAULT 'USD',
    memo                TEXT,
    -- Relative path (under the attachments root) of the rendered
    -- printable artifact. Encrypted because it contains the payee name.
    artifact_path_enc   BLOB,
    status              TEXT NOT NULL CHECK (status IN
                            ('drafted','printed','voided')),
    drafted_at          INTEGER NOT NULL,
    printed_at          INTEGER,
    voided_at           INTEGER,
    created_by          TEXT REFERENCES users(id),
    UNIQUE (settlement_id)
);
CREATE INDEX idx_check_requests_settlement ON check_requests(settlement_id);

-- ── Double-entry ledger ──────────────────────────────────────────────
-- Each row is one side of a journal entry. Entries with the same
-- `journal_id` MUST sum to zero (debit = credit). Append-only.
CREATE TABLE ledger_entries (
    id              TEXT PRIMARY KEY,
    tenant_id       TEXT NOT NULL REFERENCES tenants(id) ON DELETE RESTRICT,
    journal_id      TEXT NOT NULL,
    settlement_id   TEXT REFERENCES settlements(id),
    account         TEXT NOT NULL CHECK (account IN
                        ('deposit_liability','refund_payable',
                         'forfeited_revenue','clearing')),
    -- Signed amount in cents: positive = debit, negative = credit.
    amount_cents    INTEGER NOT NULL,
    memo            TEXT,
    occurred_at     INTEGER NOT NULL,
    created_by      TEXT REFERENCES users(id)
);
CREATE INDEX idx_ledger_journal      ON ledger_entries(journal_id);
CREATE INDEX idx_ledger_settlement   ON ledger_entries(settlement_id);
CREATE INDEX idx_ledger_tenant_time  ON ledger_entries(tenant_id, occurred_at);

CREATE TRIGGER ledger_entries_no_update
BEFORE UPDATE ON ledger_entries
BEGIN
    SELECT RAISE(ABORT, 'ledger_entries is append-only');
END;

CREATE TRIGGER ledger_entries_no_delete
BEFORE DELETE ON ledger_entries
BEGIN
    SELECT RAISE(ABORT, 'ledger_entries is append-only');
END;
