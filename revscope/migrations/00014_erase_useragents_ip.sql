-- Crawler classification is a guess, so people ended up in this table, and our
-- privacy docs promise we don't store IP addresses. Merge the per-IP rows and
-- erase the addresses. secure_delete overwrites the freed pages: without it they
-- would survive in the database file's free space.
--
-- The column and its index stay for one release. During a zero-downtime handover
-- the old binary keeps flushing after the new one has migrated, and a missing
-- column would fail its whole batch. It can therefore still write addresses, so
-- project_db_sweep.rs reruns this file whenever it finds one. That rebuilds these
-- tables as they are here, so the next migration to change them must also drop
-- the column, merging and erasing the same way.
PRAGMA secure_delete = ON;

CREATE TABLE useragents_new (
    id INTEGER PRIMARY KEY,
    useragent TEXT NOT NULL,
    operator TEXT,
    category TEXT,
    ip TEXT, -- no longer written or read, about to be dropped
    CHECK (useragent == lower(useragent))
);

INSERT INTO useragents_new (id, useragent, operator, category)
SELECT MIN(id), useragent, MAX(operator), MAX(category)
FROM useragents
GROUP BY useragent;

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
    hour, source, useragent_id, path_id, estimated_count, verified
)
SELECT cv.hour, cv.source, merged.id, cv.path_id, SUM(cv.estimated_count), cv.verified
FROM crawler_views_by_hour cv
JOIN useragents old ON old.id = cv.useragent_id
JOIN useragents_new merged ON merged.useragent = old.useragent
GROUP BY cv.hour, cv.source, merged.id, cv.path_id, cv.verified;

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

PRAGMA secure_delete = OFF;
