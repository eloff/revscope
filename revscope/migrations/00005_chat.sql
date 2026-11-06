-- Transcripts for the dashboard's Ask tab.
--
-- History lives with the project rather than in the main database so a turn
-- costs no Postgres round trip and so replaying a conversation reads from the
-- same connection that answered it. Note that project databases are
-- customer-downloadable (/v2/api/download_db), so these transcripts travel
-- with the export.
CREATE TABLE IF NOT EXISTS chat_conversations (
    id INTEGER PRIMARY KEY,
    -- The main database owns users, so this is an unenforced uuid reference.
    -- Conversations are private to the user who started them.
    user_id TEXT NOT NULL,
    -- Derived from the opening question; shown in the conversation list.
    title TEXT NOT NULL DEFAULT '',
    -- unix timestamps in seconds
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

-- The conversation list: one user's chats, most recently used first.
CREATE INDEX IF NOT EXISTS chat_conversations_user
    ON chat_conversations(user_id, updated_at DESC);

CREATE TABLE IF NOT EXISTS chat_messages (
    id INTEGER PRIMARY KEY,
    conversation_id INTEGER NOT NULL REFERENCES chat_conversations(id) ON DELETE CASCADE,
    -- 1-based, dense per conversation; replay order
    seq INTEGER NOT NULL,
    -- user, assistant, or tool
    role TEXT NOT NULL CHECK (role IN ('user', 'assistant', 'tool')),
    content TEXT NOT NULL,
    -- Set on tool rows: the SQL the model asked to run.
    tool_sql TEXT,
    -- Set on tool rows: JSON {columns, rows, has_more} or {error}. Stored so
    -- reopening a conversation redraws its tables without re-running anything.
    tool_result TEXT,
    created_at INTEGER NOT NULL,
    CHECK (seq > 0),
    UNIQUE (conversation_id, seq)
);
