-- Crawler hits keep their own path catalog so probe URLs and Cloudflare
-- dimensions do not pollute `paths` (human pageviews). Dedup is (path, hostname).
CREATE TABLE IF NOT EXISTS crawler_paths (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL,
    hostname TEXT NOT NULL,
    UNIQUE (path, hostname)
);

INSERT INTO crawler_paths (path, hostname)
SELECT DISTINCT p.path, p.hostname
FROM crawler_views_by_hour cv
JOIN paths p ON p.id = cv.path_id
WHERE cv.path_id IS NOT NULL;

CREATE TABLE crawler_views_by_hour_new (
    id INTEGER PRIMARY KEY,
    hour INTEGER NOT NULL,
    source INTEGER NOT NULL,
    useragent_id INTEGER NOT NULL REFERENCES useragents(id) ON DELETE CASCADE,
    path_id INTEGER NOT NULL REFERENCES crawler_paths(id) ON DELETE CASCADE,
    estimated_count INTEGER NOT NULL,
    CHECK (source IN (0, 1)),
    CHECK (estimated_count > 0)
);

INSERT INTO crawler_views_by_hour_new (
    id, hour, source, useragent_id, path_id, estimated_count
)
SELECT cv.id, cv.hour, cv.source, cv.useragent_id, cp.id, cv.estimated_count
FROM crawler_views_by_hour cv
JOIN paths p ON p.id = cv.path_id
JOIN crawler_paths cp ON cp.path = p.path AND cp.hostname = p.hostname
WHERE cv.path_id IS NOT NULL;

DROP TABLE crawler_views_by_hour;
ALTER TABLE crawler_views_by_hour_new RENAME TO crawler_views_by_hour;

CREATE UNIQUE INDEX IF NOT EXISTS idx_crawler_views_by_hour_key
    ON crawler_views_by_hour (hour, source, useragent_id, path_id);

CREATE INDEX IF NOT EXISTS idx_crawler_views_by_hour_path
    ON crawler_views_by_hour (path_id, hour);
