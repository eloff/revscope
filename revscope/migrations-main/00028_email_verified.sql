-- When the user last proved they read this address, by using a link that only
-- ever went to it. Password signups prove nothing, and neither do invite links,
-- which the inviter is also given.
ALTER TABLE users ADD COLUMN IF NOT EXISTS email_verified_at TIMESTAMPTZ;

ALTER TABLE password_reset_tokens
ADD COLUMN IF NOT EXISTS proves_email BOOLEAN NOT NULL DEFAULT FALSE;
