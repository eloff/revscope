-- Crawler/bot views sit beside pageviews, not inside landings.
-- landings.useragent_id is dropped in Rust when present: some databases were
-- created from an initial schema that already omitted that column.
ALTER TABLE useragents ADD COLUMN operator TEXT;
ALTER TABLE useragents ADD COLUMN category TEXT;

-- Hourly crawler hits. path_id is set only when (hostname, path) already exists
-- in paths — probe URLs share a NULL bucket per (hour, source, useragent).
-- A nullable path_id cannot be part of a WITHOUT ROWID primary key, so the
-- uniqueness lives on an expression index (same COALESCE trick as campaigns).
CREATE TABLE IF NOT EXISTS crawler_views_by_hour (
    id INTEGER PRIMARY KEY,
    hour INTEGER NOT NULL,
    source INTEGER NOT NULL,
    useragent_id INTEGER NOT NULL REFERENCES useragents(id) ON DELETE CASCADE,
    path_id INTEGER REFERENCES paths(id) ON DELETE SET NULL,
    estimated_count INTEGER NOT NULL,
    CHECK (source IN (0, 1)),
    CHECK (estimated_count > 0)
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_crawler_views_by_hour_key
    ON crawler_views_by_hour (hour, source, useragent_id, COALESCE(path_id, 0));

CREATE INDEX IF NOT EXISTS idx_crawler_views_by_hour_path
    ON crawler_views_by_hour (path_id, hour);
