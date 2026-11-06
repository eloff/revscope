-- Tracker crawler hits can be trusted when the client IP matches the operator's
-- published ranges. Cloudflare rows are already identified at the edge.
ALTER TABLE crawler_views_by_hour
    ADD COLUMN verified INTEGER NOT NULL DEFAULT 0 CHECK (verified IN (0, 1));

UPDATE crawler_views_by_hour SET verified = 1 WHERE source = 1;

DROP INDEX IF EXISTS idx_crawler_views_by_hour_key;
CREATE UNIQUE INDEX idx_crawler_views_by_hour_key
    ON crawler_views_by_hour (hour, source, useragent_id, path_id, verified);
