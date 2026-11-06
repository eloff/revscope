PRAGMA journal_mode = wal;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = OFF;

-- Prefer INTEGER PRIMARY KEY (aliases rowid) over INTEGER PRIMARY KEY AUTOINCREMENT (performs better)
-- We mostly combine unique 64bit unix nanosecond timestamps as primary keys, instead of using the client-side 96bit ids.
-- This led to a 2x speedup for some workloads since that eliminates separate time indices and internal row_ids.
-- Don't use check constraints for enum values, as we're likely to add to the enum later.
-- Prefer simple text enums as that will be more understandable to both AI and humans.

CREATE TABLE IF NOT EXISTS useragents (
    id INTEGER PRIMARY KEY,
    -- lower-cased user agent string; only populated for unrecognized browsers (most likely bots)
    useragent TEXT NOT NULL UNIQUE,
    CHECK (useragent == lower(useragent))
);

CREATE TABLE IF NOT EXISTS landings (
    -- unix timestamp in nanoseconds for the landing pageview; we only record landings for the first page
    visitor_timestamp_id INTEGER PRIMARY KEY REFERENCES pageviews(timestamp_id) ON DELETE CASCADE,
    -- denormalized session metrics, the sum of pageview durations for all pages whose visitor_timestamp_id is this landing
    session_duration_seconds INTEGER NOT NULL DEFAULT 0,
    -- the number of pages viewed in the session (not necessarily unique pages)
    session_page_count INTEGER NOT NULL DEFAULT 1,
    -- analytics consent reported by the tracker: 1 granted, 0 denied, null unknown
    consent INTEGER,
    -- external referrer
    referrer INTEGER REFERENCES referrers(id) ON DELETE SET NULL,
    campaign INTEGER REFERENCES campaigns(id) ON DELETE SET NULL,
    -- value of a click-id query param (e.g. gclid), per landing
    click_id TEXT,
    -- ai, search, social, email, campaign, affiliate (null when unclassified)
    referrer_type TEXT,
    is_google INTEGER NOT NULL, -- 1 when external referrer is Google Search
    timezone TEXT,
    language TEXT, -- ISO 639-1 code (uppercase)
    -- country, region, city are set from GeoIP
    country TEXT, -- ISO 3166-1 alpha-2 code (uppercase)
    region TEXT, -- ISO 3166-2 subdivision code (uppercase) without country e.g. QC or NY
    city TEXT, -- city name (English)
    tracker_version REAL NOT NULL,
    os TEXT, -- windows, macos, linux, android, ios, chromeos, kaios
    os_version REAL,
    browser_family TEXT, -- e.g. chrome, firefox, safari, edge
    -- set when browser_family is null (an unrecognized browser, most likely a bot)
    useragent_id INTEGER REFERENCES useragents(id) ON DELETE SET NULL,
    device TEXT NOT NULL, -- e.g. desktop, desktop_touch, mobile, tablet
    screen_width INTEGER NOT NULL DEFAULT 0,
    screen_height INTEGER NOT NULL DEFAULT 0,
    viewport_width INTEGER NOT NULL DEFAULT 0,
    viewport_height INTEGER NOT NULL DEFAULT 0,
    CHECK (os_version IS NULL OR os_version > 0),
    CHECK (language IS NULL OR (length(language) == 2 AND upper(language) = language)),
    CHECK (country IS NULL OR (length(country) == 2 AND upper(country) = country)),
    CHECK (session_duration_seconds >= 0),
    CHECK (session_page_count > 0),
    CHECK (consent IS NULL OR consent IN (0, 1)),
    CHECK (is_google IN (0, 1) AND (is_google == 0 OR referrer_type == 'search'))
);

CREATE TABLE IF NOT EXISTS paths (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL,
    hostname TEXT NOT NULL,
    deleted_at INTEGER, -- soft-delete marker, unix timestamp in seconds
    UNIQUE (path, hostname)
);

CREATE TABLE IF NOT EXISTS search_queries (
    id INTEGER PRIMARY KEY,
    query TEXT NOT NULL,
    CHECK (length(query) > 0 AND query == lower(query))
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_search_queries_query
    ON search_queries (query);

CREATE TABLE IF NOT EXISTS search_queries_by_date (
    date INTEGER NOT NULL, -- unix timestamp in seconds at UTC midnight for the date
    query_id INTEGER NOT NULL REFERENCES search_queries(id) ON DELETE CASCADE,
    position REAL NOT NULL,
    impressions INTEGER NOT NULL,
    clicks INTEGER NOT NULL,
    -- tablets grouped with mobile (matches GSC device rollup)
    mobile_clicks INTEGER NOT NULL DEFAULT 0,
    desktop_clicks INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (date, query_id),
    CHECK (impressions >= 0),
    CHECK (clicks >= 0),
    CHECK (mobile_clicks >= 0),
    CHECK (desktop_clicks >= 0),
    CHECK (position >= 0)
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS search_query_paths (
    query_id INTEGER NOT NULL REFERENCES search_queries(id) ON DELETE CASCADE,
    path_id INTEGER NOT NULL REFERENCES paths(id) ON DELETE CASCADE,
    clicks INTEGER NOT NULL, -- number of clicks on this search term from this path
    PRIMARY KEY (query_id, path_id),
    CHECK (clicks > 0)
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS referrers (
    id INTEGER PRIMARY KEY,
    referrer TEXT NOT NULL,
    deleted_at INTEGER -- soft-delete marker, unix timestamp in seconds
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_referrers_referrer
    ON referrers (referrer);

CREATE TABLE IF NOT EXISTS campaigns (
    id INTEGER PRIMARY KEY,
    source   TEXT NOT NULL DEFAULT '', -- '' instead of NULL for behavior in unique index
    medium   TEXT NOT NULL DEFAULT '',
    campaign TEXT NOT NULL,
    network TEXT,
    -- optional human label set by the user in the dashboard
    display_name TEXT,
    first_seen_at INTEGER NOT NULL, -- unix timestamp in nanoseconds; REFERENCES pageviews(timestamp_id)
    deleted_at    INTEGER, -- soft-delete marker, unix timestamp in seconds
    CHECK (length(campaign) > 0)
);

-- Natural key
CREATE UNIQUE INDEX IF NOT EXISTS idx_campaigns_attribution_tuple
    ON campaigns (
        source,
        medium,
        campaign,
        COALESCE(network, '')
    );

CREATE TABLE IF NOT EXISTS pageviews (
    -- unique unix timestamp in nanoseconds of when the page was viewed
    timestamp_id INTEGER NOT NULL PRIMARY KEY,
    -- unix timestamp in nanoseconds of the initial pageview for this user (session id).
    -- visitor_timestamp_id == timestamp_id if this is the first pageview
    visitor_timestamp_id INTEGER NOT NULL REFERENCES pageviews(timestamp_id) ON DELETE CASCADE,
    -- unix timestamp in nanoseconds of the previous pageview in this session; null for the first page view
    previous_timestamp_id INTEGER REFERENCES pageviews(timestamp_id) ON DELETE SET NULL,
    path_id INTEGER NOT NULL REFERENCES paths(id) ON DELETE CASCADE,
    duration_seconds INTEGER,
    scrolled_percent INTEGER,
    CHECK (scrolled_percent IS NULL OR scrolled_percent BETWEEN 0 AND 100)
);

CREATE INDEX IF NOT EXISTS pageviews_visitor_timestamp_id_timestamp_id
    ON pageviews(visitor_timestamp_id, timestamp_id);

CREATE TABLE IF NOT EXISTS metadata (
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    -- unix timestamp in nanoseconds; session id (landing pageview)
    visitor_timestamp_id INTEGER NOT NULL REFERENCES pageviews(timestamp_id) ON DELETE CASCADE,
    -- unix timestamp in nanoseconds; pageview the metadata was set on
    page_timestamp_id INTEGER NOT NULL REFERENCES pageviews(timestamp_id) ON DELETE CASCADE,
    PRIMARY KEY (visitor_timestamp_id, key),
    CHECK (length(key) > 0)
);

CREATE INDEX IF NOT EXISTS metadata_value
    ON metadata(value COLLATE NOCASE);

CREATE INDEX IF NOT EXISTS metadata_page_timestamp_id
    ON metadata(page_timestamp_id);

-- Index previous pageview with path_id.
-- This creates a covering index to efficiently support the costly page_transitions route.
CREATE INDEX IF NOT EXISTS pageviews_previous_timestamp_id_path_id
    ON pageviews(previous_timestamp_id, path_id)
    WHERE previous_timestamp_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS user_sessions (
    -- user_sessions maps anonymized hashed ids to sessions (visitor_timestamp_id)  
    -- for when cookie-based session tracking is not available.
    user_hash BLOB PRIMARY KEY, -- 128bit hash of ip address and user agent string
    -- unix timestamp in nanoseconds; session id (landing pageview)
    visitor_timestamp_id INTEGER NOT NULL REFERENCES pageviews(timestamp_id) ON DELETE CASCADE,
    expires_after_days INTEGER NOT NULL DEFAULT 1 -- when the session expires
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS client_page_ids_map (
    -- Maps client-generated page ids to server-assigned pageview timestamp ids for up to 1 week from page visitor_timestamp_id
    -- unix timestamp in nanoseconds; session id (landing pageview)
    visitor_timestamp_id INTEGER PRIMARY KEY REFERENCES pageviews(timestamp_id) ON DELETE CASCADE,
    page_id BLOB NOT NULL UNIQUE
);

CREATE TABLE IF NOT EXISTS event_names (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    deleted_at INTEGER -- soft-delete marker, unix timestamp in seconds
);

CREATE TABLE IF NOT EXISTS events (
    -- unique unix timestamp in nanoseconds of when the event occurred
    timestamp_id INTEGER NOT NULL PRIMARY KEY,
    event_name_id INTEGER REFERENCES event_names(id) ON DELETE CASCADE,
    -- unix timestamp in nanoseconds of the pageview the event occurred on
    page_timestamp_id INTEGER NOT NULL REFERENCES pageviews(timestamp_id) ON DELETE CASCADE,
    -- event specific metadata stored as SQLite binary JSONB via jsonb()
    metadata BLOB,
    -- json_valid flag 4 = accept JSONB binary (not JSON text)
    CHECK (metadata IS NULL OR json_valid(metadata, 4))
);

CREATE INDEX IF NOT EXISTS events_page_timestamp_event_name
    ON events(page_timestamp_id, event_name_id);

CREATE TABLE IF NOT EXISTS funnels (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()), -- unix timestamp in seconds
    updated_at INTEGER NOT NULL DEFAULT (unixepoch()), -- unix timestamp in seconds
    deleted_at INTEGER, -- soft-delete marker, unix timestamp in seconds
    CHECK (length(name) > 0)
);

CREATE TABLE IF NOT EXISTS funnel_steps (
    id INTEGER PRIMARY KEY,
    funnel_id INTEGER NOT NULL REFERENCES funnels(id) ON DELETE CASCADE,
    step_order INTEGER NOT NULL CHECK (step_order >= 0),
    step_type TEXT NOT NULL, -- pageview, payment
    -- pageview steps require a path and may optionally also match an event on that page.
    -- a payment step is optional as the final step.
    event_name_id INTEGER REFERENCES event_names(id) ON DELETE CASCADE,
    path_id INTEGER REFERENCES paths(id) ON DELETE CASCADE,
    is_landing INTEGER NOT NULL DEFAULT 0,
    payment_types TEXT, -- comma delimited list of payment_types (single, subscription, refund, chargeback)
    description TEXT,
    CHECK (is_landing IN (0, 1)),
    CHECK (
        (step_type = 'pageview' AND path_id IS NOT NULL AND payment_types IS NULL)
        OR (step_type = 'payment' AND event_name_id IS NULL AND path_id IS NULL AND payment_types IS NOT NULL AND is_landing = 0)
    ),
    UNIQUE (funnel_id, step_order)
);

CREATE INDEX IF NOT EXISTS idx_funnel_steps_funnel_order
    ON funnel_steps (funnel_id, step_order);

CREATE TABLE IF NOT EXISTS provider_objects (
    id INTEGER PRIMARY KEY,
    provider TEXT NOT NULL, -- e.g. stripe, square, shopify
    external_account_id TEXT, -- provider merchant/account/shop id when known
    object_type TEXT NOT NULL, -- provider-native object type, e.g. charge, refund, order_transaction
    provider_object_id TEXT NOT NULL,
    object_created_at INTEGER, -- unix timestamp in seconds
    object_updated_at INTEGER, -- unix timestamp in seconds
    received_at INTEGER NOT NULL, -- unix timestamp in seconds
    payload TEXT NOT NULL,
    CHECK (payload IS NOT NULL AND json_valid(payload)),
    UNIQUE (provider, object_type, provider_object_id)
);

CREATE TABLE IF NOT EXISTS subscriptions (
    id INTEGER PRIMARY KEY,
    provider_object_id INTEGER REFERENCES provider_objects(id) ON DELETE SET NULL,
    provider TEXT NOT NULL,
    provider_subscription_id TEXT NOT NULL,
    -- unix timestamp in nanoseconds; session that purchased or owns the subscription
    visitor_timestamp_id INTEGER REFERENCES pageviews(timestamp_id) ON DELETE SET NULL,
    -- unix timestamp in nanoseconds; original session for revenue attribution (may differ for shared accounts)
    attributed_visitor_timestamp_id INTEGER REFERENCES pageviews(timestamp_id) ON DELETE SET NULL,
    provider_order_id TEXT,
    provider_checkout_id TEXT,
    provider_customer_id TEXT, -- nullable provider-specific customer id
    customer_email TEXT,
    order_email TEXT,
    status TEXT,
    currency TEXT, -- ISO 4217 code (uppercase)
    interval_unit TEXT NOT NULL, -- day, week, month, year
    interval_count INTEGER NOT NULL DEFAULT 1,
    current_period_start INTEGER, 
    current_period_end INTEGER, -- unix timestamp in seconds
    next_billing_at INTEGER, -- unix timestamp in seconds
    started_at INTEGER, -- unix timestamp in seconds
    ended_at INTEGER, -- unix timestamp in seconds
    canceled_at INTEGER, -- unix timestamp in seconds
    metadata JSONB,
    CHECK (currency IS NULL OR (length(currency) == 3 AND currency == upper(currency))),
    CHECK (interval_count > 0),
    CHECK (metadata IS NULL OR json_valid(metadata)),
    UNIQUE (provider, provider_subscription_id)
);

-- Churn metrics: subscriptions churned in a time window.
CREATE INDEX IF NOT EXISTS subscriptions_churn_at
    ON subscriptions((COALESCE(ended_at, canceled_at)));

-- Churn metrics: resubscribe NOT EXISTS by customer and started_at.
CREATE INDEX IF NOT EXISTS subscriptions_customer_started_at
    ON subscriptions(
        (COALESCE(provider_customer_id, customer_email, order_email)),
        started_at
    );

-- MRR: active subscriptions with started_at before as_of.
CREATE INDEX IF NOT EXISTS subscriptions_started_at
    ON subscriptions(started_at);
CREATE INDEX IF NOT EXISTS subscriptions_visitor_timestamp_id
    ON subscriptions(visitor_timestamp_id);
CREATE INDEX IF NOT EXISTS subscriptions_attributed_visitor_timestamp_id
    ON subscriptions(attributed_visitor_timestamp_id);

CREATE TABLE IF NOT EXISTS payments (
    id INTEGER PRIMARY KEY,
    provider_object_id INTEGER REFERENCES provider_objects(id) ON DELETE SET NULL,
    provider TEXT NOT NULL,
    subscription_id INTEGER REFERENCES subscriptions(id) ON DELETE SET NULL,
    interval_unit TEXT, -- day, week, month, year; denormalized from subscriptions
    interval_count INTEGER,
    -- unix timestamp in nanoseconds; session that completed the payment
    visitor_timestamp_id INTEGER REFERENCES pageviews(timestamp_id) ON DELETE SET NULL,
    -- unix timestamp in nanoseconds; original session for revenue attribution (may differ for shared accounts)
    attributed_visitor_timestamp_id INTEGER REFERENCES pageviews(timestamp_id) ON DELETE SET NULL,
    -- denormalized session metrics up to this payment (null when unattributed)
    session_duration_seconds INTEGER,
    session_page_count INTEGER,

    provider_payment_id TEXT NOT NULL, 
    provider_object_type TEXT, -- provider-native object type represented by this ledger row
    provider_status TEXT,
    provider_order_id TEXT,
    provider_checkout_id TEXT,
    provider_invoice_id TEXT,
    provider_refund_id TEXT,
    provider_dispute_id TEXT,
    parent_provider_payment_id TEXT,
    provider_customer_id TEXT,
    customer_email TEXT,
    order_email TEXT,

    payment_type TEXT NOT NULL, -- single, subscription, refund, chargeback
    status TEXT NOT NULL DEFAULT 'ok', -- pending, ok, failed, canceled
    -- integer thousandths of the currency unit, negative for refund and chargeback
    amount INTEGER NOT NULL,
    -- normalized amount for monthly recurring payments, in integer thousandths of the currency unit
    mrr_amount INTEGER,
    currency TEXT NOT NULL,
    fee_amount INTEGER, -- integer thousandths of the currency unit
    net_amount INTEGER, -- integer thousandths of the currency unit
    settlement_currency TEXT, -- ISO 4217 code (uppercase)
    settlement_amount INTEGER, -- integer thousandths of the settlement currency
    exchange_rate REAL,

    paid_at INTEGER NOT NULL, -- unix timestamp in seconds
    created_at INTEGER NOT NULL, -- unix timestamp in seconds
    metadata JSONB,
    CHECK (
        (payment_type IN ('refund', 'chargeback') AND amount < 0 AND (net_amount IS NULL OR net_amount < 0))
        OR (payment_type NOT IN ('refund', 'chargeback') AND amount >= 0 AND (net_amount IS NULL OR net_amount >= 0))
    ),
    CHECK (
        length(currency) == 3 AND currency == upper(currency)
    ),
    CHECK (settlement_currency IS NULL OR (length(settlement_currency) == 3 AND settlement_currency == upper(settlement_currency))),
    CHECK (interval_count IS NULL OR interval_count > 0),
    CHECK (exchange_rate IS NULL OR exchange_rate > 0),
    CHECK (metadata IS NULL OR json_valid(metadata)),
    CHECK (session_duration_seconds IS NULL OR session_duration_seconds >= 0),
    CHECK (session_page_count IS NULL OR session_page_count > 0),
    UNIQUE (provider, provider_payment_id)
);

CREATE INDEX IF NOT EXISTS payments_visitor_timestamp_id ON payments(visitor_timestamp_id);
CREATE INDEX IF NOT EXISTS payments_attributed_visitor_timestamp_id ON payments(attributed_visitor_timestamp_id);
CREATE INDEX IF NOT EXISTS payments_subscription_id ON payments(subscription_id);
CREATE INDEX IF NOT EXISTS payments_paid_at_id ON payments(paid_at, id);
-- Revenue metrics: ok payments in a paid_at range.
CREATE INDEX IF NOT EXISTS payments_ok_paid_at
    ON payments(paid_at)
    WHERE status = 'ok';
-- Revenue recurring EXISTS + MRR latest payment per subscription.
CREATE INDEX IF NOT EXISTS payments_subscription_recurring
    ON payments(subscription_id, paid_at)
    WHERE status = 'ok' AND payment_type = 'subscription';
