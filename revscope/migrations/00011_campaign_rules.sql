-- User-defined matchers that assign unclassified landings to a campaign.
-- A rule matches a stored query parameter. Several rules on one campaign are
-- alternatives.
CREATE TABLE IF NOT EXISTS campaign_rules (
    id INTEGER PRIMARY KEY,
    campaign_id INTEGER NOT NULL REFERENCES campaigns(id) ON DELETE CASCADE,
    param_key TEXT NOT NULL CHECK (length(param_key) > 0),
    param_value TEXT NOT NULL CHECK (length(param_value) > 0),
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE INDEX IF NOT EXISTS idx_campaign_rules_campaign
    ON campaign_rules (campaign_id);

CREATE UNIQUE INDEX IF NOT EXISTS idx_campaign_rules_match
    ON campaign_rules (campaign_id, param_key, param_value COLLATE NOCASE);
