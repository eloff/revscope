-- Marks a subscription period that ended because it wasn't paid for: the
-- provider gave up collecting a renewal, or a trial ended without a payment.
-- The account then moves onto the free plan in a new period, and this one
-- keeps the plan and event volume it can pay to restore. A period the customer
-- cancelled, or that a plan change replaced, ends without the mark.
ALTER TABLE account_subscriptions
    ADD COLUMN ended_unpaid BOOLEAN NOT NULL DEFAULT FALSE;

ALTER TABLE account_subscriptions
    ADD CONSTRAINT account_subscriptions_ended_unpaid_has_ended
        CHECK (NOT ended_unpaid OR ended_at IS NOT NULL);
