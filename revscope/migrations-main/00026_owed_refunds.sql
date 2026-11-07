-- Refunds a plan change owes, so that a failed refund or a crash leaves a trace. A
-- row with settled_at NULL needs someone to look at it. account_id has no
-- foreign key because the money is still owed if the account is deleted.
CREATE TABLE IF NOT EXISTS owed_refunds (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    account_id UUID NOT NULL,
    provider TEXT NOT NULL,
    subscription_id TEXT NOT NULL,
    -- 'refund' is money the plan change owes, recorded before the provider is called.
    -- 'account_credit' is what's left of a customer's Stripe balance after leaving for
    -- the free plan: it's still spendable there, so paying it means clearing it too.
    kind TEXT NOT NULL DEFAULT 'refund' CHECK (kind IN ('refund', 'account_credit')),
    amount_cents INTEGER NOT NULL CHECK (amount_cents > 0),
    refunded_cents INTEGER NOT NULL DEFAULT 0,
    -- Set if ending the subscription returned an error, which doesn't mean it stayed
    -- active: the response may just have been lost. The provider's detail is in the logs.
    provider_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    settled_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS owed_refunds_unsettled_idx
ON owed_refunds (created_at)
WHERE settled_at IS NULL;
