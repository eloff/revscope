-- Deleting an account retires its project names like deleting a project does,
-- but the account isn't there to take them back. Its retirements stay, with no
-- account, so no account can claim those names again.
ALTER TABLE retired_project_names ALTER COLUMN account_id DROP NOT NULL;

ALTER TABLE retired_project_names
DROP CONSTRAINT IF EXISTS retired_project_names_account_id_fkey;

ALTER TABLE retired_project_names
ADD CONSTRAINT retired_project_names_account_id_fkey
FOREIGN KEY (account_id) REFERENCES accounts (id) ON DELETE SET NULL;
