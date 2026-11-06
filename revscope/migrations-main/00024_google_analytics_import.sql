-- Credentials an admin has connected but not started an import with yet. A row
-- starts holding only oauth_state while the admin is away at Google's consent
-- screen. Starting an import moves the credentials onto ga_imports.
CREATE TABLE IF NOT EXISTS ga_import_connections (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id INTEGER NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    -- Cleared rather than cascaded when the user goes, so the expiry sweep still
    -- gets to revoke the refresh token; no lookup matches a NULL.
    created_by UUID REFERENCES users (id) ON DELETE SET NULL,
    oauth_state TEXT UNIQUE,
    -- Set for a service account. Without it, encrypted_secret is an OAuth refresh token.
    encrypted_client_email BYTEA,
    encrypted_secret BYTEA,
    -- The Google account behind a refresh token. Revoking one token ends that
    -- account's whole grant to our client, so this tells us when nothing else needs it.
    oauth_subject TEXT,
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS ga_import_connections_expires_at_idx
ON ga_import_connections (expires_at);

-- The imported rows themselves are in the project's SQLite database, under `id`.
CREATE TABLE IF NOT EXISTS ga_imports (
    id SERIAL PRIMARY KEY,
    project_id INTEGER NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    property_id TEXT NOT NULL,
    property_name TEXT NOT NULL,
    start_date DATE NOT NULL,
    end_date DATE NOT NULL,
    hostnames TEXT[] NOT NULL DEFAULT '{}', -- empty imports every hostname
    -- Cleared once the import stops, so nothing outlives the job.
    encrypted_client_email BYTEA,
    encrypted_secret BYTEA,
    oauth_subject TEXT,
    -- running or paused (out of GA quota, resumes by itself) until it stops as
    -- completed, cancelled or failed.
    status TEXT NOT NULL DEFAULT 'running'
        CHECK (status IN ('running', 'paused', 'completed', 'cancelled', 'failed')),
    chunks_done INTEGER NOT NULL DEFAULT 0,
    chunks_total INTEGER NOT NULL,
    failed_attempts INTEGER NOT NULL DEFAULT 0,
    resume_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    error_text TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    finished_at TIMESTAMPTZ,
    CHECK (end_date >= start_date)
);

CREATE INDEX IF NOT EXISTS ga_imports_project_idx ON ga_imports (project_id);

-- One import at a time per project: they share one property's GA quota.
CREATE UNIQUE INDEX IF NOT EXISTS ga_imports_one_active_idx
ON ga_imports (project_id)
WHERE status IN ('running', 'paused');

CREATE INDEX IF NOT EXISTS ga_imports_due_idx
ON ga_imports (resume_at)
WHERE status IN ('running', 'paused');
