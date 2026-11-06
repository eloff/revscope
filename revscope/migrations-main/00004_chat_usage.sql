-- One row per completed Ask turn, used to rate limit the AI gateway spend.
--
-- The transcripts themselves live in the project database; only what a turn
-- cost is recorded here, because the limits are per account and per user and
-- so cannot be answered from a single project.
CREATE TABLE IF NOT EXISTS chat_usage (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    account_id UUID NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    project_id INTEGER NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    model TEXT NOT NULL,
    prompt_tokens INTEGER NOT NULL DEFAULT 0,
    completion_tokens INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- The monthly account token budget.
CREATE INDEX IF NOT EXISTS chat_usage_account_created_idx
ON chat_usage (account_id, created_at DESC);

-- The hourly per-user turn limit.
CREATE INDEX IF NOT EXISTS chat_usage_user_created_idx
ON chat_usage (user_id, created_at DESC);
