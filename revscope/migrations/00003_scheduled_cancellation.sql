-- Separates a cancellation request from the date the subscription actually
-- stops.
--
-- Providers report both, and `canceled_at` is the request. Stripe documents
-- that for `cancel_at_period_end` its `canceled_at` "will reflect the time of
-- the most recent update request, not the end of the subscription period", and
-- Polar, Square, Lemon Squeezy and Dodo all draw the same distinction. Treating
-- the request as terminal removed MRR and recorded churn up to a full billing
-- period early, for a subscription the customer had already paid to keep.

-- When a cancellation that has been requested takes effect, for a subscription
-- still running out the period it paid for. Providers clear it when the
-- cancellation is reversed, and it stays set once `ended_at` confirms the end.
ALTER TABLE subscriptions ADD COLUMN cancel_at INTEGER; -- unix timestamp in seconds
ALTER TABLE subscription_history ADD COLUMN cancel_at INTEGER; -- unix timestamp in seconds

-- Churn is keyed off the effective end, so the 00001 index over
-- COALESCE(ended_at, canceled_at) no longer covers the query.
DROP INDEX IF EXISTS subscriptions_churn_at;
CREATE INDEX IF NOT EXISTS subscriptions_churn_at
    ON subscriptions((COALESCE(ended_at, cancel_at, canceled_at)));
