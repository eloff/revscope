-- Turn account_subscriptions into a history: one row per subscription period.
--
-- 00008 gave each account exactly one subscription row and left plan_type and
-- events_monthly_limit on accounts. That records only what an account is on
-- right now, so the billing screen had no way to say what it was on before.
--
-- Periods are half-open [started_at, ended_at); the current one has ended_at
-- NULL. Entitlements move onto the period so each row is a complete record of
-- the terms in force at the time. The open period is the current subscription,
-- which is what every entitlement read resolves to.
--
-- Non-overlap across an account's periods is enforced in the backend write
-- helpers (see account_subscription.rs), not with a GiST exclusion constraint.

BEGIN;

ALTER TABLE account_subscriptions DROP CONSTRAINT account_subscriptions_pkey;

ALTER TABLE account_subscriptions ADD COLUMN id UUID NOT NULL DEFAULT uuidv7();
ALTER TABLE account_subscriptions ADD PRIMARY KEY (id);

ALTER TABLE account_subscriptions
    ADD COLUMN plan_type plan_type,
    ADD COLUMN events_monthly_limit INTEGER,
    ADD COLUMN started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    ADD COLUMN ended_at TIMESTAMPTZ;

UPDATE account_subscriptions s
SET plan_type = a.plan_type,
    events_monthly_limit = a.events_monthly_limit
FROM accounts a
WHERE a.id = s.account_id;

ALTER TABLE account_subscriptions ALTER COLUMN plan_type SET NOT NULL;

ALTER TABLE account_subscriptions
    ADD CONSTRAINT account_subscriptions_period_order
        CHECK (ended_at IS NULL OR ended_at > started_at);

-- An account has at most one current subscription. Entitlement reads join on
-- `ended_at IS NULL` and expect a single row.
CREATE UNIQUE INDEX account_subscriptions_open_idx
ON account_subscriptions (account_id)
WHERE ended_at IS NULL;

CREATE INDEX account_subscriptions_account_idx
ON account_subscriptions (account_id, started_at DESC);

-- Polar ids repeat across an account's history once a subscription ends, so
-- uniqueness now only holds among current periods.
DROP INDEX account_subscriptions_polar_subscription_id_idx;
CREATE UNIQUE INDEX account_subscriptions_polar_subscription_id_idx
ON account_subscriptions (polar_subscription_id)
WHERE polar_subscription_id IS NOT NULL AND ended_at IS NULL;

ALTER TABLE accounts
    DROP COLUMN plan_type,
    DROP COLUMN events_monthly_limit;

COMMIT;
