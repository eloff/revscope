CREATE TYPE plan_type AS ENUM ('starter', 'growth', 'enterprise', 'founder');

CREATE TYPE integration_type AS ENUM ('google_search_console', 'stripe', 'shopify', 'square', 'lemon_squeezy', 'polar', 'dodo');

CREATE TABLE IF NOT EXISTS errors (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    project_id INTEGER NOT NULL,
    error_type TEXT NOT NULL,
    is_server_error BOOLEAN NOT NULL DEFAULT FALSE,
    error_text TEXT,
    path TEXT,
    data TEXT
);

CREATE TABLE IF NOT EXISTS accounts (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    name TEXT NOT NULL UNIQUE CHECK (char_length(trim(name)) > 4),
    plan_type plan_type NOT NULL,
    -- NULL when the account is not on a trial (e.g. founder/enterprise).
    -- Self-serve signup will set this to NOW() + 14 days once payments are enabled.
    trial_ends_at TIMESTAMPTZ,
    -- Account-wide monthly event cap shared across all projects. NULL = unlimited.
    events_monthly_limit INTEGER
);

CREATE TABLE IF NOT EXISTS projects (
    id SERIAL PRIMARY KEY,
    account_id UUID NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    name TEXT NOT NULL UNIQUE CHECK (name ~ '^[a-z0-9_]{3,}$'),
    payment_success_path TEXT
);

-- One row per successful ingest flush batch for a project.
CREATE TABLE IF NOT EXISTS projects_last_ingest (
    project_id INTEGER NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    started_at BIGINT NOT NULL, -- unix nanoseconds when the flush batch started
    finished_at BIGINT NOT NULL, -- unix nanoseconds when the flush batch finished
    events_inserted INTEGER NOT NULL, -- pageviews + events inserted in the batch
    PRIMARY KEY (project_id, started_at)
);

CREATE TABLE IF NOT EXISTS integrations (
    id SERIAL PRIMARY KEY,
    project_id INTEGER NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    external_account_id TEXT,
    encrypted_api_key BYTEA,
    encrypted_secret BYTEA,
    integration_type integration_type NOT NULL,
    disabled_at TIMESTAMPTZ,
    -- Denormalized scheduler state, maintained by complete_integration_sync().
    -- Lets the sync scheduler poll with a plain index scan on this table instead
    -- of re-deriving state from integration_syncs history on every tick.
    last_sync_completed_at TIMESTAMPTZ, -- any outcome; throttles how often we re-check this integration
    last_sync_success_at TIMESTAMPTZ, -- started_at of the last successful sync; drives the incremental "since" cursor
    last_full_sync_success_at TIMESTAMPTZ -- started_at of the last successful full sync; drives full vs incremental
);

-- Drives the sync scheduler's "is this integration due?" check.
CREATE INDEX IF NOT EXISTS integrations_due_for_sync_idx
ON integrations (last_sync_completed_at)
WHERE disabled_at IS NULL;

-- One integration row per (project, type); the settings page upserts on this key.
CREATE UNIQUE INDEX IF NOT EXISTS integrations_project_type_idx
ON integrations (project_id, integration_type);

CREATE TABLE IF NOT EXISTS integration_syncs (
    integration_id INTEGER NOT NULL REFERENCES integrations (id) ON DELETE CASCADE,
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ,
    sync_kind TEXT NOT NULL DEFAULT 'incremental' CHECK (sync_kind IN ('incremental', 'full')),
    records_inserted INTEGER NOT NULL DEFAULT 0,
    records_updated INTEGER NOT NULL DEFAULT 0,
    records_deleted INTEGER NOT NULL DEFAULT 0,
    error_text TEXT, -- success = error_text is null
    PRIMARY KEY (integration_id, started_at)
);

CREATE TABLE IF NOT EXISTS users (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    account_id UUID NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    name TEXT NOT NULL CHECK (char_length(trim(name)) >= 2),
    email TEXT NOT NULL,
    original_email TEXT,
    hashed_password BYTEA,
    last_login_at TIMESTAMPTZ,
    login_provider INTEGER NOT NULL DEFAULT 0
);

CREATE UNIQUE INDEX IF NOT EXISTS users_email_idx ON users (email);

CREATE TABLE IF NOT EXISTS exclude_users (
    account_id UUID NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    -- Recorded for possible future use; exclusions apply to the whole account.
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- Network prefix for exclusion matching (host bits zeroed by cidr).
    network CIDR NOT NULL,
    user_agent TEXT NOT NULL CHECK (user_agent <> ''),
    expires_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (account_id, network, user_agent)
);

CREATE INDEX IF NOT EXISTS exclude_users_account_expiry_idx
ON exclude_users (account_id, expires_at);

CREATE TABLE IF NOT EXISTS sessions (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS password_reset_tokens (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS password_reset_tokens_user_id_idx
ON password_reset_tokens (user_id);

CREATE INDEX IF NOT EXISTS password_reset_tokens_expires_at_idx
ON password_reset_tokens (expires_at);

CREATE TABLE IF NOT EXISTS user_settings (
    user_id UUID PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    timezone TEXT NOT NULL DEFAULT 'UTC'
);

-- Demo account and project
INSERT INTO accounts (id, name, plan_type, trial_ends_at, events_monthly_limit)
VALUES ('c5460626-00b6-4d98-b95d-ac0bacc24296', 'demo.com', 'founder', NULL, NULL);

INSERT INTO projects (account_id, name)
VALUES (
    'c5460626-00b6-4d98-b95d-ac0bacc24296',
    'demo'
);

-- Example account and project
INSERT INTO accounts (id, name, plan_type, trial_ends_at, events_monthly_limit)
VALUES ('01960626-00b6-4d98-b95d-ac0bacc24296', 'example.com', 'founder', NULL, NULL);

INSERT INTO projects (account_id, name)
VALUES (
    '01960626-00b6-4d98-b95d-ac0bacc24296',
    'example'
);