-- At most one open history version per subscription.
--
-- The previous partial index only sped up "find the open version"; without
-- UNIQUE a failure between closing the old version and inserting its successor
-- could leave zero open versions, and a bug or concurrent writer could leave
-- more than one. Appends now run in a transaction; this constraint makes the
-- invariant hold even if that discipline slips.

DROP INDEX IF EXISTS subscription_history_open;
CREATE UNIQUE INDEX IF NOT EXISTS subscription_history_open
    ON subscription_history(subscription_id)
    WHERE effective_to IS NULL;
