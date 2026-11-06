-- Ask transcripts for the public demo project.
--
-- These live in the main database rather than the project SQLite so they
-- survive demo DB resets, stay out of the unauthenticated demo download, and
-- can be audited. Conversations are keyed by an opaque owner_key: the client
-- IP for anonymous visitors (shared history on shared IPs is intentional) or
-- the signed-in user_id so logged-in accounts keep private threads.
CREATE TABLE IF NOT EXISTS demo_chat_conversations (
    id BIGSERIAL PRIMARY KEY,
    owner_key TEXT NOT NULL,
    -- Set when the request had a resolvable client IP; used for audit.
    client_ip INET,
    title TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS demo_chat_conversations_owner_updated_idx
ON demo_chat_conversations (owner_key, updated_at DESC);

CREATE TABLE IF NOT EXISTS demo_chat_messages (
    id BIGSERIAL PRIMARY KEY,
    conversation_id BIGINT NOT NULL REFERENCES demo_chat_conversations (id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('user', 'assistant', 'tool')),
    content TEXT NOT NULL,
    tool_sql TEXT,
    tool_result TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (seq > 0),
    UNIQUE (conversation_id, seq)
);

-- Anonymous demo Ask spend, keyed by client IP so clearing any client state
-- cannot reset the budget.
CREATE TABLE IF NOT EXISTS demo_chat_usage (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    client_ip INET NOT NULL,
    model TEXT NOT NULL,
    prompt_tokens INTEGER NOT NULL DEFAULT 0,
    completion_tokens INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS demo_chat_usage_ip_created_idx
ON demo_chat_usage (client_ip, created_at DESC);
