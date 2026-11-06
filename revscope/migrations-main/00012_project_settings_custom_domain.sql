ALTER TABLE project_settings
    ADD COLUMN IF NOT EXISTS custom_domain TEXT;

-- One project may claim a hostname; ACME will only issue for names in this column
-- (plus the hardcoded platform hosts).
CREATE UNIQUE INDEX IF NOT EXISTS project_settings_custom_domain_key
    ON project_settings (custom_domain)
    WHERE custom_domain IS NOT NULL;
