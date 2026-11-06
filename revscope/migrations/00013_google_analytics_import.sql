-- Comments go inside each CREATE because the schema dump agents read comes from
-- sqlite_master, which drops whatever comes before the CREATE.

CREATE TABLE IF NOT EXISTS imports (
    -- One row per import of history from Google Analytics 4; the rows of every
    -- imported_* table hang off it. GA only reports daily aggregates, so those
    -- tables can't be joined to pageviews, landings or events: an imported session
    -- has no individual pageviews, journey or revenue. Each table counts a date and
    -- one dimension on its own, so never combine two imported_* tables to answer
    -- "pages where country = US". `date` in them is the GA property's calendar date,
    -- read as a day in the project's timezone. Add them to tracked figures for the
    -- same period to get totals. Sessions are this schema's visitors.
    id INTEGER PRIMARY KEY,
    source TEXT NOT NULL, -- google_analytics
    source_name TEXT NOT NULL, -- GA property name
    start_date TEXT NOT NULL, -- YYYY-MM-DD, inclusive
    end_date TEXT NOT NULL, -- YYYY-MM-DD, inclusive
    CHECK (end_date >= start_date)
);

CREATE TABLE IF NOT EXISTS imported_data_settings (
    -- Whether reports add imported data to tracked data. No row means they do.
    id INTEGER PRIMARY KEY CHECK (id = 1),
    include_in_reports INTEGER NOT NULL,
    CHECK (include_in_reports IN (0, 1))
);

CREATE TABLE IF NOT EXISTS imported_totals (
    import_id INTEGER NOT NULL REFERENCES imports(id) ON DELETE CASCADE,
    date TEXT NOT NULL,
    sessions INTEGER NOT NULL,
    pageviews INTEGER NOT NULL,
    -- sessions minus engaged_sessions are bounces
    engaged_sessions INTEGER NOT NULL,
    engagement_seconds INTEGER NOT NULL,
    PRIMARY KEY (import_id, date)
) WITHOUT ROWID;

CREATE INDEX IF NOT EXISTS imported_totals_date ON imported_totals(date);

CREATE TABLE IF NOT EXISTS imported_pages (
    import_id INTEGER NOT NULL REFERENCES imports(id) ON DELETE CASCADE,
    date TEXT NOT NULL,
    path_id INTEGER NOT NULL REFERENCES paths(id) ON DELETE CASCADE,
    sessions INTEGER NOT NULL,
    pageviews INTEGER NOT NULL,
    engagement_seconds INTEGER NOT NULL,
    PRIMARY KEY (import_id, date, path_id)
) WITHOUT ROWID;

CREATE INDEX IF NOT EXISTS imported_pages_date ON imported_pages(date);

CREATE TABLE IF NOT EXISTS imported_landing_pages (
    import_id INTEGER NOT NULL REFERENCES imports(id) ON DELETE CASCADE,
    date TEXT NOT NULL,
    path_id INTEGER NOT NULL REFERENCES paths(id) ON DELETE CASCADE,
    sessions INTEGER NOT NULL,
    engaged_sessions INTEGER NOT NULL,
    engagement_seconds INTEGER NOT NULL,
    PRIMARY KEY (import_id, date, path_id)
) WITHOUT ROWID;

CREATE INDEX IF NOT EXISTS imported_landing_pages_date ON imported_landing_pages(date);

CREATE TABLE IF NOT EXISTS imported_referrers (
    -- Sessions without a campaign. A session with one is in imported_campaigns instead.
    import_id INTEGER NOT NULL REFERENCES imports(id) ON DELETE CASCADE,
    date TEXT NOT NULL,
    -- 0 for direct, which has no referrers row
    referrer_id INTEGER NOT NULL,
    channel TEXT NOT NULL, -- search, ai, social, campaigns, referrers, direct
    sessions INTEGER NOT NULL,
    PRIMARY KEY (import_id, date, referrer_id, channel)
) WITHOUT ROWID;

CREATE INDEX IF NOT EXISTS imported_referrers_date ON imported_referrers(date);

CREATE TABLE IF NOT EXISTS imported_campaigns (
    import_id INTEGER NOT NULL REFERENCES imports(id) ON DELETE CASCADE,
    date TEXT NOT NULL,
    campaign_id INTEGER NOT NULL REFERENCES campaigns(id) ON DELETE CASCADE,
    sessions INTEGER NOT NULL,
    PRIMARY KEY (import_id, date, campaign_id)
) WITHOUT ROWID;

CREATE INDEX IF NOT EXISTS imported_campaigns_date ON imported_campaigns(date);

CREATE TABLE IF NOT EXISTS imported_geo (
    -- '' stands for an unknown region or city, so the primary key can hold it.
    import_id INTEGER NOT NULL REFERENCES imports(id) ON DELETE CASCADE,
    date TEXT NOT NULL,
    country TEXT NOT NULL, -- ISO 3166-1 alpha-2 code (uppercase)
    region TEXT NOT NULL, -- GA's region name, e.g. New York, not an ISO 3166-2 code like landings.region
    city TEXT NOT NULL,
    sessions INTEGER NOT NULL,
    PRIMARY KEY (import_id, date, country, region, city)
) WITHOUT ROWID;

CREATE INDEX IF NOT EXISTS imported_geo_date ON imported_geo(date);

CREATE TABLE IF NOT EXISTS imported_technology (
    -- Same values as landings.device, landings.browser_family and landings.os where
    -- GA's have an equivalent; 'unknown' when GA doesn't know.
    import_id INTEGER NOT NULL REFERENCES imports(id) ON DELETE CASCADE,
    date TEXT NOT NULL,
    device TEXT NOT NULL,
    browser_family TEXT NOT NULL,
    os TEXT NOT NULL,
    sessions INTEGER NOT NULL,
    PRIMARY KEY (import_id, date, device, browser_family, os)
) WITHOUT ROWID;

CREATE INDEX IF NOT EXISTS imported_technology_date ON imported_technology(date);

CREATE TABLE IF NOT EXISTS imported_events (
    -- Custom events only, not the ones GA collects by itself like page_view and scroll.
    import_id INTEGER NOT NULL REFERENCES imports(id) ON DELETE CASCADE,
    date TEXT NOT NULL,
    event_name_id INTEGER NOT NULL REFERENCES event_names(id) ON DELETE CASCADE,
    path_id INTEGER NOT NULL REFERENCES paths(id) ON DELETE CASCADE,
    sessions INTEGER NOT NULL,
    event_count INTEGER NOT NULL,
    PRIMARY KEY (import_id, date, event_name_id, path_id)
) WITHOUT ROWID;

CREATE INDEX IF NOT EXISTS imported_events_date ON imported_events(date);
