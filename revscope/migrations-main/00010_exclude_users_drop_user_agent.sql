-- Exclusions are now network-only; clear existing rows (which keyed on
-- user_agent) and drop that column from the primary key.

BEGIN;

DELETE FROM exclude_users;

ALTER TABLE exclude_users DROP CONSTRAINT exclude_users_pkey;
ALTER TABLE exclude_users DROP COLUMN user_agent;
ALTER TABLE exclude_users ADD PRIMARY KEY (account_id, network);

COMMIT;
