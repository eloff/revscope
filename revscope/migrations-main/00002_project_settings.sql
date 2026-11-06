CREATE TABLE IF NOT EXISTS project_settings (
    project_id INTEGER PRIMARY KEY REFERENCES projects (id) ON DELETE CASCADE,
    timezone TEXT NOT NULL DEFAULT 'UTC'
);

ALTER TABLE user_settings DROP COLUMN timezone;
