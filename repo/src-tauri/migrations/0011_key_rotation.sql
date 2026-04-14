-- Key-rotation state tracking.
--
-- `key_rotations` records the lifecycle of a rotation run: which old
-- label is being retired, which new label is taking over, and the
-- final status. A partial unique index enforces AT MOST ONE
-- in-progress rotation at any time so a crashed rotation can always
-- be resumed exactly where it left off.
--
-- `key_rotation_progress` carries a per-field cursor. Each batch of a
-- rotation re-encrypts rows with id > last_id (ordered by id) and
-- advances `last_id` IN THE SAME TRANSACTION as the row updates,
-- making the cursor consistent with the data at every commit.

PRAGMA foreign_keys = ON;

CREATE TABLE key_rotations (
    id              TEXT PRIMARY KEY,
    old_label       TEXT NOT NULL,
    new_label       TEXT NOT NULL,
    started_at      INTEGER NOT NULL,
    completed_at    INTEGER,
    status          TEXT NOT NULL CHECK (status IN
                        ('in_progress','completed','aborted')),
    error_message   TEXT,
    CHECK (new_label <> old_label)
);

-- Exactly one rotation may be in_progress at any time.
CREATE UNIQUE INDEX idx_key_rotations_one_in_progress
    ON key_rotations(status) WHERE status = 'in_progress';

CREATE TABLE key_rotation_progress (
    rotation_id     TEXT NOT NULL REFERENCES key_rotations(id) ON DELETE CASCADE,
    table_name      TEXT NOT NULL,
    column_name     TEXT NOT NULL,
    last_id         TEXT,                           -- cursor; NULL before first row
    completed_at    INTEGER,                        -- set once the field is fully rotated
    PRIMARY KEY (rotation_id, table_name, column_name)
);
CREATE INDEX idx_key_rotation_progress_pending
    ON key_rotation_progress(rotation_id, completed_at);
