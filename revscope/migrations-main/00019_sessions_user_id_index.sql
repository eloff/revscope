-- A password reset deletes the user's sessions by user_id, and deleting a user
-- cascades to them the same way; without this both scan every session.
CREATE INDEX IF NOT EXISTS sessions_user_id_idx
ON sessions (user_id);
