-- Google Search Console is synced one report day at a time, committing each day
-- separately, so an interrupted backfill needs to know where to resume. The
-- project database can't answer that: a day with no search traffic writes no
-- rows and is indistinguishable from a day that was never fetched.
ALTER TABLE integrations
    -- Most recent provider report date fully written for this integration.
    -- Google Search Console stores US Pacific calendar dates here.
    ADD COLUMN synced_through_date DATE;
