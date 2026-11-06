-- Names of deleted projects. A project's name is its tracker ingest path, so a
-- site still embedding a deleted project's script would otherwise send its
-- traffic to whichever account claims the name next. Only the account that
-- deleted the name may take it again.
CREATE TABLE IF NOT EXISTS retired_project_names (
    name TEXT PRIMARY KEY,
    account_id UUID NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    retired_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
