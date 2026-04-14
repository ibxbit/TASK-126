-- Dispute resolution additions to the claims domain.
-- Scope:
--   1. Extend `claims` with dispute-lifecycle columns and a wider
--      status domain supporting two-party confirmation.
--   2. Record per-party responses in `claim_party_responses`.
--   3. Record reopen events with approval trail; enforce at most one
--      reopen per claim.

PRAGMA foreign_keys = OFF;

-- ── Rebuild `claims` with extended status + category domains ─────────
ALTER TABLE claims RENAME TO claims_legacy_0003;

CREATE TABLE claims (
    id                      TEXT PRIMARY KEY,
    tenant_id               TEXT NOT NULL REFERENCES tenants(id) ON DELETE RESTRICT,
    case_id                 TEXT REFERENCES move_out_cases(id) ON DELETE SET NULL,
    parcel_id               TEXT REFERENCES parcels(id)        ON DELETE SET NULL,
    claim_number            TEXT NOT NULL,
    kind                    TEXT NOT NULL CHECK (kind IN
                                ('parcel_ownership','deposit_deduction')),
    category                TEXT NOT NULL CHECK (category IN
                                ('damage','cleaning','unpaid_rent',
                                 'missing_item','parcel_ownership','other')),
    unit_address            TEXT,
    keywords                TEXT,   -- space-separated, normalized lowercase
    amount_cents            INTEGER NOT NULL DEFAULT 0 CHECK (amount_cents >= 0),

    -- Two-party dispute resolution
    claimant_user_id        TEXT NOT NULL REFERENCES users(id),
    respondent_user_id      TEXT REFERENCES users(id),

    status                  TEXT NOT NULL CHECK (status IN
                                ('draft','submitted','under_review',
                                 'confirmed','resolved','contested',
                                 'auto_cancelled','withdrawn','rejected_final',
                                 'reopened')),

    submitted_at            INTEGER,
    response_deadline_unix  INTEGER,   -- 72h after submission
    resolved_at             INTEGER,
    reopened_count          INTEGER NOT NULL DEFAULT 0 CHECK (reopened_count <= 1),

    opened_at               INTEGER NOT NULL,
    closed_at               INTEGER,
    description_enc         BLOB,

    created_at              INTEGER NOT NULL,
    updated_at              INTEGER NOT NULL,
    created_by              TEXT REFERENCES users(id),
    updated_by              TEXT REFERENCES users(id),
    UNIQUE (tenant_id, claim_number)
);

-- Migrate legacy rows. Legacy `status` maps into the new domain; every
-- legacy claim is treated as a deposit_deduction unless its category
-- already matched a parcel-specific kind (none did in 0001).
INSERT INTO claims (
    id, tenant_id, case_id, parcel_id, claim_number, kind, category,
    unit_address, keywords, amount_cents,
    claimant_user_id, respondent_user_id,
    status, submitted_at, response_deadline_unix, resolved_at, reopened_count,
    opened_at, closed_at, description_enc,
    created_at, updated_at, created_by, updated_by
)
SELECT
    id, tenant_id, case_id, NULL, claim_number,
    'deposit_deduction',
    CASE category
        WHEN 'damage'       THEN 'damage'
        WHEN 'cleaning'     THEN 'cleaning'
        WHEN 'unpaid_rent'  THEN 'unpaid_rent'
        WHEN 'missing_item' THEN 'missing_item'
        ELSE 'other'
    END,
    NULL, NULL, amount_cents,
    COALESCE(created_by, (SELECT id FROM users LIMIT 1)),
    NULL,
    CASE status
        WHEN 'new'       THEN 'draft'
        WHEN 'in_review' THEN 'under_review'
        WHEN 'accepted'  THEN 'resolved'
        WHEN 'rejected'  THEN 'rejected_final'
        WHEN 'reopened'  THEN 'reopened'
        WHEN 'closed'    THEN 'resolved'
        ELSE 'draft'
    END,
    NULL, NULL,
    CASE WHEN status IN ('accepted','rejected','closed') THEN closed_at END,
    CASE WHEN status = 'reopened' THEN 1 ELSE 0 END,
    opened_at, closed_at, description_enc,
    created_at, updated_at, created_by, updated_by
FROM claims_legacy_0003;

DROP TABLE claims_legacy_0003;

CREATE INDEX idx_claims_tenant_status ON claims(tenant_id, status);
CREATE INDEX idx_claims_case          ON claims(case_id);
CREATE INDEX idx_claims_parcel        ON claims(parcel_id);
CREATE INDEX idx_claims_opened_at     ON claims(tenant_id, opened_at);
CREATE INDEX idx_claims_deadline      ON claims(status, response_deadline_unix);
CREATE INDEX idx_claims_matching      ON claims(tenant_id, kind, category, unit_address);

PRAGMA foreign_keys = ON;

-- ── Two-party confirmation responses ─────────────────────────────────
CREATE TABLE claim_party_responses (
    id              TEXT PRIMARY KEY,
    claim_id        TEXT NOT NULL REFERENCES claims(id) ON DELETE CASCADE,
    party_role      TEXT NOT NULL CHECK (party_role IN ('claimant','respondent')),
    user_id         TEXT NOT NULL REFERENCES users(id),
    response        TEXT NOT NULL CHECK (response IN ('accept','reject')),
    responded_at    INTEGER NOT NULL,
    notes_enc       BLOB,
    UNIQUE (claim_id, party_role)
);
CREATE INDEX idx_claim_responses_claim ON claim_party_responses(claim_id);

-- Append-only: responses are statements of record.
CREATE TRIGGER claim_party_responses_no_update
BEFORE UPDATE ON claim_party_responses
BEGIN
    SELECT RAISE(ABORT, 'claim_party_responses is append-only');
END;

CREATE TRIGGER claim_party_responses_no_delete
BEFORE DELETE ON claim_party_responses
BEGIN
    SELECT RAISE(ABORT, 'claim_party_responses is append-only');
END;

-- ── Reopen audit (at most one per claim, enforced by claims.reopened_count) ──
CREATE TABLE claim_reopens (
    id                  TEXT PRIMARY KEY,
    claim_id            TEXT NOT NULL UNIQUE REFERENCES claims(id) ON DELETE CASCADE,
    requested_by        TEXT NOT NULL REFERENCES users(id),
    approved_by         TEXT NOT NULL REFERENCES users(id),
    approved_at         INTEGER NOT NULL,
    reason_enc          BLOB
);
CREATE INDEX idx_claim_reopens_claim ON claim_reopens(claim_id);
