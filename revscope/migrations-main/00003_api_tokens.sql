-- Long-lived, project-scoped tokens for programmatic API access (the MCP
-- server). Unlike sessions these are used outside a browser, so they are bound
-- to one project instead of a whole account. Only the sha256 of the token is
-- stored: the plaintext is returned to the caller once, when it is minted.
CREATE TABLE IF NOT EXISTS api_tokens (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    token_hash BYTEA NOT NULL,
    -- What the token unlocks. 'query' grants the read-only /v2/api/query and
    -- /v2/api/schema endpoints for its project.
    kind TEXT NOT NULL CHECK (kind IN ('query')),
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    project_id INTEGER NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Every request authorized by a token looks it up by hash.
CREATE UNIQUE INDEX IF NOT EXISTS api_tokens_token_hash_idx
ON api_tokens (token_hash);

CREATE INDEX IF NOT EXISTS api_tokens_expires_at_idx
ON api_tokens (expires_at);
