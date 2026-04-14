-- Local analytics: typed event dimensions, funnels, daily roll-ups,
-- and a self-contained A/B experiment engine.

PRAGMA foreign_keys = ON;

-- ── Extend `events` with analytics dimensions ────────────────────────
ALTER TABLE events ADD COLUMN category     TEXT
    CHECK (category IN ('impression','click','completion','conversion'));
ALTER TABLE events ADD COLUMN funnel       TEXT;     -- nullable: funnel name
ALTER TABLE events ADD COLUMN funnel_step  INTEGER;  -- nullable: 1-based
ALTER TABLE events ADD COLUMN duration_ms  INTEGER;  -- quality metric
ALTER TABLE events ADD COLUMN success      INTEGER CHECK (success IN (0,1));
ALTER TABLE events ADD COLUMN experiment_id TEXT;
ALTER TABLE events ADD COLUMN variant_id    TEXT;

CREATE INDEX idx_events_category_time
    ON events(tenant_id, category, occurred_at);
CREATE INDEX idx_events_funnel_step
    ON events(tenant_id, funnel, funnel_step, occurred_at);
CREATE INDEX idx_events_session
    ON events(session_id, occurred_at);
CREATE INDEX idx_events_experiment
    ON events(experiment_id, variant_id, occurred_at);

-- ── Funnel definitions (ordered step → event kind mapping) ───────────
CREATE TABLE funnels (
    id              TEXT PRIMARY KEY,
    tenant_id       TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    description     TEXT,
    enabled         INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0,1)),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    UNIQUE (tenant_id, name)
);

CREATE TABLE funnel_steps (
    funnel_id       TEXT NOT NULL REFERENCES funnels(id) ON DELETE CASCADE,
    step_no         INTEGER NOT NULL CHECK (step_no >= 1),
    event_kind      TEXT NOT NULL,
    label           TEXT NOT NULL,
    PRIMARY KEY (funnel_id, step_no),
    UNIQUE (funnel_id, event_kind)
);

-- ── Daily roll-ups for fast dashboard queries ────────────────────────
-- Aggregator job (or a simple INSERT … ON CONFLICT DO UPDATE on each
-- track call) keeps this table fresh; dashboards read here, not from
-- the raw `events` table, for top-line metrics.
CREATE TABLE daily_event_aggregates (
    tenant_id       TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    day_unix        INTEGER NOT NULL,            -- midnight UTC of the day
    category        TEXT NOT NULL CHECK (category IN
                        ('impression','click','completion','conversion')),
    kind            TEXT NOT NULL,
    count_total     INTEGER NOT NULL DEFAULT 0,
    count_success   INTEGER NOT NULL DEFAULT 0,
    sum_duration_ms INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (tenant_id, day_unix, category, kind)
);
CREATE INDEX idx_dea_tenant_day ON daily_event_aggregates(tenant_id, day_unix);

-- ── A/B experiments ──────────────────────────────────────────────────
CREATE TABLE experiments (
    id              TEXT PRIMARY KEY,
    tenant_id       TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    description     TEXT,
    start_at_unix   INTEGER NOT NULL,
    end_at_unix     INTEGER NOT NULL,
    enabled         INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0,1)),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    UNIQUE (tenant_id, name),
    CHECK (end_at_unix > start_at_unix)
);
CREATE INDEX idx_experiments_active ON experiments(tenant_id, enabled, start_at_unix, end_at_unix);

CREATE TABLE experiment_variants (
    id              TEXT PRIMARY KEY,
    experiment_id   TEXT NOT NULL REFERENCES experiments(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    -- Weight in basis points (out of 10_000) so percentages are exact.
    weight_bp       INTEGER NOT NULL CHECK (weight_bp >= 0 AND weight_bp <= 10000),
    UNIQUE (experiment_id, name)
);
CREATE INDEX idx_experiment_variants_exp ON experiment_variants(experiment_id);

-- One assignment per (experiment, subject). Sticky for the lifetime of
-- the experiment so a user always sees the same variant.
CREATE TABLE experiment_assignments (
    experiment_id   TEXT NOT NULL REFERENCES experiments(id) ON DELETE CASCADE,
    subject_id      TEXT NOT NULL,            -- typically user_id
    variant_id      TEXT NOT NULL REFERENCES experiment_variants(id) ON DELETE RESTRICT,
    assigned_at     INTEGER NOT NULL,
    PRIMARY KEY (experiment_id, subject_id)
);
CREATE INDEX idx_experiment_assignments_variant
    ON experiment_assignments(experiment_id, variant_id);
