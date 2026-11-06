-- Move RevScope billing off the accounts table onto a 1:1 subscription row.
--
-- accounts keeps product entitlements (plan_type, trial_ends_at,
-- events_monthly_limit). account_subscriptions owns locked-in commercial
-- terms and the Polar lifecycle cache that used to live on accounts.
--
-- One subscription per account for now; account_id is the primary key so
-- entitlement and checkout joins stay a single equality.

BEGIN;

CREATE TABLE account_subscriptions (
    account_id UUID PRIMARY KEY REFERENCES accounts (id) ON DELETE CASCADE,
    billing_period billing_period NOT NULL,
    regular_price_cents INTEGER NOT NULL,
    actual_price_cents INTEGER NOT NULL,
    discount_ends_at TIMESTAMPTZ,
    polar_customer_id TEXT,
    polar_subscription_id TEXT,
    -- Polar's status verbatim (`trialing`, `active`, `past_due`, ...).
    subscription_status TEXT,
    -- End of the period the account has already paid for.
    current_period_end TIMESTAMPTZ,
    -- Polar's modified_at for the subscription state stored above. Webhook
    -- deliveries can arrive out of order, so a retried older event is dropped.
    subscription_updated_at TIMESTAMPTZ,
    CONSTRAINT account_subscriptions_regular_price_non_negative
        CHECK (regular_price_cents >= 0),
    CONSTRAINT account_subscriptions_actual_price_non_negative
        CHECK (actual_price_cents >= 0),
    CONSTRAINT account_subscriptions_actual_price_lte_regular
        CHECK (actual_price_cents <= regular_price_cents)
);

INSERT INTO account_subscriptions (
    account_id,
    billing_period,
    regular_price_cents,
    actual_price_cents,
    discount_ends_at,
    polar_customer_id,
    polar_subscription_id,
    subscription_status,
    current_period_end,
    subscription_updated_at
)
SELECT
    id,
    billing_period,
    regular_price_cents,
    actual_price_cents,
    discount_ends_at,
    polar_customer_id,
    polar_subscription_id,
    subscription_status,
    current_period_end,
    subscription_updated_at
FROM accounts;

CREATE UNIQUE INDEX account_subscriptions_polar_subscription_id_idx
ON account_subscriptions (polar_subscription_id)
WHERE polar_subscription_id IS NOT NULL;

CREATE INDEX account_subscriptions_polar_customer_id_idx
ON account_subscriptions (polar_customer_id)
WHERE polar_customer_id IS NOT NULL;

DROP INDEX IF EXISTS accounts_polar_subscription_id_idx;
DROP INDEX IF EXISTS accounts_polar_customer_id_idx;

ALTER TABLE accounts
    DROP CONSTRAINT IF EXISTS accounts_regular_price_non_negative,
    DROP CONSTRAINT IF EXISTS accounts_actual_price_non_negative,
    DROP CONSTRAINT IF EXISTS accounts_actual_price_lte_regular;

ALTER TABLE accounts
    DROP COLUMN billing_period,
    DROP COLUMN regular_price_cents,
    DROP COLUMN actual_price_cents,
    DROP COLUMN discount_ends_at,
    DROP COLUMN polar_customer_id,
    DROP COLUMN polar_subscription_id,
    DROP COLUMN subscription_status,
    DROP COLUMN current_period_end,
    DROP COLUMN subscription_updated_at;

COMMIT;
