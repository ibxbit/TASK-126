-- Local share-package registry.
-- The encrypted ZIP itself lives under the attachments root. This
-- table holds metadata + a verifier for the access password.

PRAGMA foreign_keys = ON;

CREATE TABLE share_packages (
    id                      TEXT PRIMARY KEY,
    tenant_id               TEXT NOT NULL REFERENCES tenants(id) ON DELETE RESTRICT,
    created_by              TEXT NOT NULL REFERENCES users(id),

    -- Encrypted relative path (under the attachments root) of the
    -- AES-encrypted ZIP package. Encrypted via FieldCipher with AAD
    -- "share_packages.artifact_path_enc:<id>".
    artifact_path_enc       BLOB NOT NULL,

    -- Argon2id hash of the access password and the salt used. The
    -- password itself is never persisted; only the hash is kept so
    -- access attempts can be verified offline.
    password_hash           TEXT NOT NULL,
    password_salt           BLOB NOT NULL,

    -- Non-sensitive labels for UI listing.
    recipient_label         TEXT,
    contents_summary        TEXT,

    created_at_unix         INTEGER NOT NULL,
    expires_at_unix         INTEGER NOT NULL CHECK (expires_at_unix > created_at_unix),
    revoked_at_unix         INTEGER,

    last_accessed_at_unix   INTEGER,
    access_count            INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_share_packages_tenant_active
    ON share_packages(tenant_id, expires_at_unix, revoked_at_unix);
CREATE INDEX idx_share_packages_expiry
    ON share_packages(expires_at_unix);
