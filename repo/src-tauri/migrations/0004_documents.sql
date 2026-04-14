-- Document management: versions, tagging, resumable chunked uploads.
-- The base `attachments` table from migration 0001 continues to act as
-- the metadata row per logical document (one row per attachment,
-- regardless of how many versions it has).

PRAGMA foreign_keys = ON;

-- ── Per-version records ──────────────────────────────────────────────
CREATE TABLE attachment_versions (
    id                  TEXT PRIMARY KEY,
    attachment_id       TEXT NOT NULL REFERENCES attachments(id) ON DELETE CASCADE,
    version_no          INTEGER NOT NULL CHECK (version_no >= 1),
    -- Encrypted relative path under the attachments root, per-version.
    relative_path_enc   BLOB NOT NULL,
    byte_size           INTEGER NOT NULL CHECK (byte_size >= 0),
    sha256_hex          TEXT NOT NULL,
    created_at          INTEGER NOT NULL,
    created_by          TEXT REFERENCES users(id),
    UNIQUE (attachment_id, version_no)
);
CREATE INDEX idx_attachment_versions_attachment
    ON attachment_versions(attachment_id, version_no);
CREATE INDEX idx_attachment_versions_sha256 ON attachment_versions(sha256_hex);

-- Append-only: versions are never mutated or deleted in place.
CREATE TRIGGER attachment_versions_no_update
BEFORE UPDATE ON attachment_versions
BEGIN
    SELECT RAISE(ABORT, 'attachment_versions is append-only');
END;

-- ── Tagging (M:N) ────────────────────────────────────────────────────
-- Tag labels are NOT sensitive and intentionally stored in plaintext
-- so SQL can drive tag-based search cheaply.
CREATE TABLE attachment_tags (
    attachment_id   TEXT NOT NULL REFERENCES attachments(id) ON DELETE CASCADE,
    tag             TEXT NOT NULL,
    created_at      INTEGER NOT NULL,
    created_by      TEXT REFERENCES users(id),
    PRIMARY KEY (attachment_id, tag)
);
CREATE INDEX idx_attachment_tags_tag ON attachment_tags(tag);

-- ── Chunked upload sessions ──────────────────────────────────────────
CREATE TABLE upload_sessions (
    id                  TEXT PRIMARY KEY,
    tenant_id           TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    entity_kind         TEXT NOT NULL CHECK (entity_kind IN
                            ('case','settlement','parcel','claim','resident')),
    entity_id           TEXT NOT NULL,
    -- Target (eventual) display name and mime, encrypted to match the
    -- attachments row that will be minted on finalize.
    display_name_enc    BLOB NOT NULL,
    mime_type           TEXT NOT NULL,
    total_bytes         INTEGER NOT NULL CHECK (total_bytes >= 0),
    chunk_size          INTEGER NOT NULL CHECK (chunk_size > 0),
    chunk_count         INTEGER NOT NULL CHECK (chunk_count >= 0),
    expected_sha256_hex TEXT NOT NULL,
    -- Staging directory relative path under the staging root.
    staging_rel_path    TEXT NOT NULL,
    -- If the session is completing an update to an existing attachment,
    -- this is set; otherwise a new attachment is minted on finalize.
    target_attachment_id TEXT REFERENCES attachments(id) ON DELETE CASCADE,
    status              TEXT NOT NULL CHECK (status IN
                            ('in_progress','finalized','aborted')),
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL,
    created_by          TEXT REFERENCES users(id)
);
CREATE INDEX idx_upload_sessions_tenant_status
    ON upload_sessions(tenant_id, status);
CREATE INDEX idx_upload_sessions_entity
    ON upload_sessions(entity_kind, entity_id);

-- ── Received chunks per session ──────────────────────────────────────
CREATE TABLE upload_chunks (
    session_id      TEXT NOT NULL REFERENCES upload_sessions(id) ON DELETE CASCADE,
    chunk_index     INTEGER NOT NULL CHECK (chunk_index >= 0),
    byte_size       INTEGER NOT NULL CHECK (byte_size >= 0),
    sha256_hex      TEXT NOT NULL,
    received_at     INTEGER NOT NULL,
    PRIMARY KEY (session_id, chunk_index)
);
