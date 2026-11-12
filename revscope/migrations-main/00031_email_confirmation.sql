-- When the user followed the link emailed to confirm their address, which turns
-- on the features new accounts wait for. Unlike email_verified_at, it says
-- nothing about who else holds the user's password or tokens, so it never lets
-- a Google or Apple sign-in join the user.
ALTER TABLE users ADD COLUMN IF NOT EXISTS email_confirmed_at TIMESTAMPTZ;

-- Links that confirm a user reads their email address. Kept apart from
-- password_reset_tokens so that following one can never set a password.
CREATE TABLE IF NOT EXISTS email_verification_tokens (
    token_hash BYTEA PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS email_verification_tokens_user_id_idx
ON email_verification_tokens (user_id);

CREATE INDEX IF NOT EXISTS email_verification_tokens_expires_at_idx
ON email_verification_tokens (expires_at);
