-- Unverified crawler user-agents keep the client IP so the same UA from
-- different hosts does not collapse into one row. Verified rows leave ip NULL.
-- Rebuild useragents to replace UNIQUE(useragent) with (useragent, COALESCE(ip, '')).
CREATE TABLE useragents_new (
    id INTEGER PRIMARY KEY,
    useragent TEXT NOT NULL,
    operator TEXT,
    category TEXT,
    ip TEXT,
    CHECK (useragent == lower(useragent))
);

INSERT INTO useragents_new (id, useragent, operator, category)
SELECT id, useragent, operator, category FROM useragents;

CREATE TABLE crawler_views_by_hour_new (
    id INTEGER PRIMARY KEY,
    hour INTEGER NOT NULL,
    source INTEGER NOT NULL,
    useragent_id INTEGER NOT NULL REFERENCES useragents_new(id) ON DELETE CASCADE,
    path_id INTEGER NOT NULL REFERENCES crawler_paths(id) ON DELETE CASCADE,
    estimated_count INTEGER NOT NULL,
    verified INTEGER NOT NULL DEFAULT 0 CHECK (verified IN (0, 1)),
    CHECK (source IN (0, 1)),
    CHECK (estimated_count > 0)
);

INSERT INTO crawler_views_by_hour_new (
    id, hour, source, useragent_id, path_id, estimated_count, verified
)
SELECT id, hour, source, useragent_id, path_id, estimated_count, verified
FROM crawler_views_by_hour;

DROP TABLE crawler_views_by_hour;
DROP TABLE useragents;
ALTER TABLE useragents_new RENAME TO useragents;
ALTER TABLE crawler_views_by_hour_new RENAME TO crawler_views_by_hour;

CREATE UNIQUE INDEX idx_useragents_useragent_ip
    ON useragents (useragent, COALESCE(ip, ''));

CREATE UNIQUE INDEX idx_crawler_views_by_hour_key
    ON crawler_views_by_hour (hour, source, useragent_id, path_id, verified);

CREATE INDEX idx_crawler_views_by_hour_path
    ON crawler_views_by_hour (path_id, hour);
