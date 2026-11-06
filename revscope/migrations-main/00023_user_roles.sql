-- Admins see and change billing and add and manage the account's users.
-- Everyone else uses the analytics of every project on the account, or only
-- those listed in user_projects when all_projects is false.
--
-- Every user predating roles could already do everything on their account,
-- so the default makes them admins. It stays TRUE because servers still on
-- the previous release keep inserting users without naming the column while
-- a deploy rolls out, and their signups must not create an account with no
-- admin. Current code always sets is_admin explicitly.
ALTER TABLE users
ADD COLUMN IF NOT EXISTS is_admin BOOLEAN NOT NULL DEFAULT TRUE;

ALTER TABLE users
ADD COLUMN IF NOT EXISTS all_projects BOOLEAN NOT NULL DEFAULT TRUE;

ALTER TABLE users DROP CONSTRAINT IF EXISTS users_admin_all_projects;

ALTER TABLE users
ADD CONSTRAINT users_admin_all_projects CHECK (NOT is_admin OR all_projects);

CREATE TABLE IF NOT EXISTS user_projects (
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    project_id INTEGER NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    PRIMARY KEY (user_id, project_id)
);

-- For the cascade when a project is deleted.
CREATE INDEX IF NOT EXISTS user_projects_project_id_idx
ON user_projects (project_id);

-- Removing a user must not hand their Ask spend back to the account's monthly
-- budget, so their usage rows outlive them.
ALTER TABLE chat_usage ALTER COLUMN user_id DROP NOT NULL;

ALTER TABLE chat_usage DROP CONSTRAINT IF EXISTS chat_usage_user_id_fkey;

ALTER TABLE chat_usage
ADD CONSTRAINT chat_usage_user_id_fkey
FOREIGN KEY (user_id) REFERENCES users (id) ON DELETE SET NULL;
