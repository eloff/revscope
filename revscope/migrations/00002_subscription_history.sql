-- Append-only subscription state history.
--
-- `subscriptions` stays a stable one-row-per-provider-subscription anchor
-- holding identity, attribution, and a denormalized copy of the current state,
-- so `payments.subscription_id` and the renewal-detection queries keep working
-- unchanged. Time-varying plan and lifecycle state is appended here instead of
-- being overwritten, which is what makes upgrades and downgrades
-- reconstructable.
--
-- Rows carry closed [effective_from, effective_to) ranges so "state as of T" is
-- a plain indexed range scan rather than a window function, and a new row is
-- only appended when a tracked field actually changes (see `fingerprint`), so a
-- subscription that renews for years without a plan change keeps a single row.

-- The recurring price is what makes an upgrade visible before the next invoice
-- arrives; MRR previously had to be inferred from the last payment amount.
ALTER TABLE subscriptions ADD COLUMN provider_price_id TEXT;
ALTER TABLE subscriptions ADD COLUMN plan_name TEXT;
ALTER TABLE subscriptions ADD COLUMN quantity INTEGER;
-- integer thousandths of the currency unit, charged once per billing interval
ALTER TABLE subscriptions ADD COLUMN plan_amount INTEGER;
-- plan_amount normalized to a month, in integer thousandths
ALTER TABLE subscriptions ADD COLUMN mrr_amount INTEGER;

CREATE TABLE IF NOT EXISTS subscription_history (
    id INTEGER PRIMARY KEY,
    subscription_id INTEGER NOT NULL REFERENCES subscriptions(id) ON DELETE CASCADE,
    -- 1-based, dense per subscription
    version INTEGER NOT NULL,
    -- unix timestamps in seconds; effective_to IS NULL for the current version
    effective_from INTEGER NOT NULL,
    effective_to INTEGER,
    -- when the sync observed the change, which can lag effective_from
    observed_at INTEGER NOT NULL,
    -- created, upgrade, downgrade, interval_change, quantity_change,
    -- status_change, canceled, resumed, ended
    change_kind TEXT NOT NULL,
    status TEXT,
    provider_price_id TEXT,
    plan_name TEXT,
    quantity INTEGER,
    currency TEXT, -- ISO 4217 code (uppercase)
    plan_amount INTEGER, -- integer thousandths of the currency unit
    mrr_amount INTEGER, -- integer thousandths, normalized to a month
    -- mrr_amount of the preceding version, denormalized so MRR movement
    -- queries need no self join
    prev_mrr_amount INTEGER,
    interval_unit TEXT NOT NULL, -- day, week, month, year
    interval_count INTEGER NOT NULL DEFAULT 1,
    canceled_at INTEGER, -- unix timestamp in seconds
    ended_at INTEGER, -- unix timestamp in seconds
    provider_object_id INTEGER REFERENCES provider_objects(id) ON DELETE SET NULL,
    -- hash of the tracked fields, used to suppress no-op appends on resync
    fingerprint TEXT NOT NULL,
    CHECK (version > 0),
    CHECK (interval_count > 0),
    CHECK (effective_to IS NULL OR effective_to > effective_from),
    CHECK (currency IS NULL OR (length(currency) == 3 AND currency == upper(currency))),
    UNIQUE (subscription_id, version)
);

-- MRR as of a point in time: seek to a subscription's version covering it.
CREATE INDEX IF NOT EXISTS subscription_history_subscription_effective_from
    ON subscription_history(subscription_id, effective_from);

-- MRR movement: versions that took effect within a window.
CREATE INDEX IF NOT EXISTS subscription_history_effective_from
    ON subscription_history(effective_from);

-- Locating the open version to close on append.
CREATE INDEX IF NOT EXISTS subscription_history_open
    ON subscription_history(subscription_id)
    WHERE effective_to IS NULL;
