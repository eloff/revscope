-- Date-scope Search Console query→page clicks so the Search Terms landing-paths
-- column follows the dashboard date range (same window as clicks/impressions).
-- No production data to migrate: drop the undated aggregate table.
DROP TABLE IF EXISTS search_query_paths;

CREATE TABLE IF NOT EXISTS search_query_paths_by_date (
    date INTEGER NOT NULL, -- unix timestamp in seconds at UTC midnight for the date
    query_id INTEGER NOT NULL REFERENCES search_queries(id) ON DELETE CASCADE,
    path_id INTEGER NOT NULL REFERENCES paths(id) ON DELETE CASCADE,
    clicks INTEGER NOT NULL,
    PRIMARY KEY (date, query_id, path_id),
    CHECK (clicks > 0)
) WITHOUT ROWID;
