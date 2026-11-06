-- Sessions and password reset tokens were keyed by the plaintext id their
-- holder presents, so read access to this database was enough to take over an
-- account. Now only the sha256 of the token is stored, as for api_tokens.
-- Existing rows are dropped rather than converted: everyone logs in again, and
-- outstanding reset and invite links stop working.
DROP TABLE IF EXISTS sessions;

CREATE TABLE sessions (
    token_hash BYTEA PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    expires_at TIMESTAMPTZ NOT NULL
);

-- A password reset deletes the user's sessions by user_id, and deleting a user
-- cascades to them the same way; without this both scan every session.
CREATE INDEX sessions_user_id_idx
ON sessions (user_id);

CREATE INDEX sessions_expires_at_idx
ON sessions (expires_at);

DROP TABLE IF EXISTS password_reset_tokens;

CREATE TABLE password_reset_tokens (
    token_hash BYTEA PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX password_reset_tokens_user_id_idx
ON password_reset_tokens (user_id);

CREATE INDEX password_reset_tokens_expires_at_idx
ON password_reset_tokens (expires_at);
