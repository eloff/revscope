-- Bumped whenever an integration's credentials or account change, so a sync that
-- started with the old ones can't overwrite the reset scheduler state.
ALTER TABLE integrations
    ADD COLUMN IF NOT EXISTS credentials_version BIGINT NOT NULL DEFAULT 0;

-- The version a sync ran with, so its outcome isn't reported as the new
-- credential's status.
ALTER TABLE integration_syncs
    ADD COLUMN IF NOT EXISTS credentials_version BIGINT NOT NULL DEFAULT 0;
