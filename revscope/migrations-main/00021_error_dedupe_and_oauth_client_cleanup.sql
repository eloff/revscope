-- Tracker JS errors are deduplicated per project, per UTC day: a repeat of the
-- same error text bumps `count` instead of adding a row, and the row keeps the
-- path it was first seen on. Every column added here is nullable or has a
-- constant default, so none of them rewrites the table, which may already be
-- large. The index below still scans it once, blocking writes to `errors` (not
-- reads) while it builds.
ALTER TABLE errors ADD COLUMN IF NOT EXISTS count BIGINT NOT NULL DEFAULT 1;

-- Both set only on deduplicated (javascript) rows. `fingerprint` is a hash of
-- error_text, computed by the insert, because the text itself can exceed the
-- btree's per-entry size limit.
ALTER TABLE errors ADD COLUMN IF NOT EXISTS error_day DATE;
ALTER TABLE errors ADD COLUMN IF NOT EXISTS fingerprint TEXT;

-- Partial, so it holds only deduplicated rows. An insert must repeat the WHERE
-- clause in its ON CONFLICT target for Postgres to use this index.
CREATE UNIQUE INDEX IF NOT EXISTS errors_project_day_fingerprint_idx
ON errors (project_id, error_day, fingerprint) WHERE error_day IS NOT NULL;

-- Registrations that never redeemed a code are deleted after a day. Clients
-- registered before this column existed are assumed to be in use, so none of
-- them is deleted out from under a user who already connected.
ALTER TABLE oauth_clients ADD COLUMN IF NOT EXISTS token_issued_at TIMESTAMPTZ;

UPDATE oauth_clients SET token_issued_at = created_at WHERE token_issued_at IS NULL;

CREATE INDEX IF NOT EXISTS oauth_clients_unused_created_at_idx
ON oauth_clients (created_at) WHERE token_issued_at IS NULL;
