-- Stripe billing ids for RevScope's own subscriptions.
--
-- Checkout is created against a Stripe product chosen from the account's plan
-- (Starter or Growth), with a minted Price for the locked-in amount. Stripe
-- then owns the billing lifecycle and reports it back over the webhook, which
-- writes the columns below. Polar columns stay; CHECKOUT_PROVIDER selects
-- which provider a new checkout uses.

ALTER TABLE account_subscriptions
    ADD COLUMN stripe_customer_id TEXT,
    ADD COLUMN stripe_subscription_id TEXT;

-- Stripe ids repeat across an account's history once a subscription ends, so
-- uniqueness only holds among current periods, matching Polar.
CREATE UNIQUE INDEX account_subscriptions_stripe_subscription_id_idx
ON account_subscriptions (stripe_subscription_id)
WHERE stripe_subscription_id IS NOT NULL AND ended_at IS NULL;

CREATE INDEX account_subscriptions_stripe_customer_id_idx
ON account_subscriptions (stripe_customer_id)
WHERE stripe_customer_id IS NOT NULL;
