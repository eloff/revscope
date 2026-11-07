-- When the latest invite was sent, which a re-invite moves. Until now nothing
-- recorded that, so everyone who hasn't accepted gets a fresh start from here.
ALTER TABLE users ADD COLUMN IF NOT EXISTS invited_at TIMESTAMPTZ;

UPDATE users SET invited_at = NOW() WHERE hashed_password IS NULL;
