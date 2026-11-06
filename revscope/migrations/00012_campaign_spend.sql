-- Spend entered by hand for a campaign. Dates are project-local calendar
-- dates, inclusive. A `once` entry is a single amount on start_date; a `daily`
-- entry is `amount` on every day from start_date through end_date. Amounts are
-- thousandths of the project currency, like revenue.
CREATE TABLE IF NOT EXISTS campaign_spend (
    id INTEGER PRIMARY KEY,
    campaign_id INTEGER NOT NULL REFERENCES campaigns(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('once', 'daily')),
    start_date TEXT NOT NULL,
    end_date TEXT NOT NULL CHECK (end_date >= start_date),
    amount INTEGER NOT NULL CHECK (amount > 0),
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE INDEX IF NOT EXISTS idx_campaign_spend_campaign
    ON campaign_spend (campaign_id);
