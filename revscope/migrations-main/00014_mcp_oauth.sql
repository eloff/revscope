-- The hosted MCP server at /v2/mcp authenticates with its own kind of token.
-- Unlike a query token it is handed out by an OAuth flow the agent runs in a
-- browser, and it lasts a year, because reconnecting means walking the user
-- through consent again.
ALTER TABLE api_tokens DROP CONSTRAINT IF EXISTS api_tokens_kind_check;

ALTER TABLE api_tokens
ADD CONSTRAINT api_tokens_kind_check CHECK (kind IN ('query', 'mcp'));

-- OAuth clients that registered themselves through RFC 7591. They are public
-- clients (no secret): the authorization code is bound to a PKCE challenge and
-- to one of these redirect URIs instead.
CREATE TABLE IF NOT EXISTS oauth_clients (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    -- Shown on the approval page so the user knows what is asking.
    client_name TEXT NOT NULL,
    -- Exact-match redirect URIs, in registration order.
    redirect_uris TEXT [] NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
