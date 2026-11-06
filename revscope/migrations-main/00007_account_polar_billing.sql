-- Polar billing state for RevScope's own subscriptions.
--
-- Checkout is created by the backend against a Polar product chosen from the
-- account's plan and billing period, with the locked-in price passed as an
-- ad-hoc price. Polar then owns the billing lifecycle and reports it back over
-- the webhook, which writes the columns below.
--
-- subscription_status is Polar's status verbatim (`trialing`, `active`,
-- `past_due`, `canceled`, `unpaid`, ...) rather than an enum, so a status Polar
-- adds later cannot fail the webhook.

ALTER TABLE accounts
    ADD COLUMN polar_customer_id TEXT,
    ADD COLUMN polar_subscription_id TEXT,
    ADD COLUMN subscription_status TEXT,
    -- End of the period the account has already paid for.
    ADD COLUMN current_period_end TIMESTAMPTZ,
    -- Polar's modified_at for the subscription state stored above. Webhook
    -- deliveries can arrive out of order, so a retried older event is dropped
    -- rather than rolling the account back to a stale status.
    ADD COLUMN subscription_updated_at TIMESTAMPTZ;

-- One account per Polar subscription; the webhook looks accounts up by it.
CREATE UNIQUE INDEX accounts_polar_subscription_id_idx
ON accounts (polar_subscription_id)
WHERE polar_subscription_id IS NOT NULL;

CREATE INDEX accounts_polar_customer_id_idx
ON accounts (polar_customer_id)
WHERE polar_customer_id IS NOT NULL;

-- Paid invoices for RevScope subscriptions, as reported by Polar order events.
-- Kept for support and reconciliation; Polar remains the source of truth.
CREATE TABLE IF NOT EXISTS account_orders (
    polar_order_id TEXT PRIMARY KEY,
    account_id UUID NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    polar_subscription_id TEXT,
    -- Polar order status (`paid`, `refunded`, `pending`, ...).
    status TEXT NOT NULL,
    paid BOOLEAN NOT NULL,
    billing_reason TEXT,
    -- Charged amount including tax, in the order's currency's minor units.
    total_amount_cents INTEGER NOT NULL,
    currency TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS account_orders_account_idx
ON account_orders (account_id, created_at DESC);
