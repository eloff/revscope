//! Visitors and revenue by every filterable dimension, for the compact panels
//! under the overview chart.
//!
//! Each breakdown applies every active segment filter except its own, so a
//! panel keeps listing the alternatives to what is selected and one more can
//! be added from it. Doing that in SQL would take a query per filtered
//! dimension, so instead one scan of the period's landings, merged with scans
//! of their pageviews and revenue, evaluates the filters here per session.
//! Only the event filter, which no panel breaks down, stays in SQL.
//!
//! Visitors are sessions that landed in the period. Revenue is what those
//! sessions went on to pay, renewals excluded, the same attribution the
//! audience and revenue-source views use.

use fxhash::{FxHashMap, FxHashSet};
use rusqlite::params_from_iter;
use rusqlite::types::ToSql;
use std::fmt::Write;

use serde::Serialize;
use smallvec::SmallVec;

use crate::auth::{AuthError, map_row_error};
use crate::country_region::country_name_for_country;
use crate::db::{ProjectConnection, SqlCompactStr};
use crate::ga_import::ImportedDays;
use crate::landing_labels::{browser_family_name, device_name, os_name};
use crate::page_revenue::page_revenue_select;
use crate::referrer::SourceGroup;
use crate::segment_filter::{LandingAttribute, LandingFacts, SegmentFilters, SourceFilter};
use crate::util::NANOS_PER_SECOND;

/// Rows kept per breakdown, by visitors and again by revenue; a panel lists
/// them all and sorts them on either column without another request.
pub(crate) const BREAKDOWN_ROWS: usize = 100;

#[derive(Debug, Serialize, PartialEq)]
pub(crate) struct CampaignFields {
    source: String,
    medium: String,
    campaign: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    network: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    display_name: Option<String>,
}

#[derive(Debug, Serialize, PartialEq)]
pub(crate) struct BreakdownRow {
    /// The segment filter value selecting this row: a country code, a
    /// `source` value, a `paths.id`, or a [`LandingAttribute`] value.
    pub(crate) value: String,
    pub(crate) label: String,
    /// Present on campaign source rows, which the dashboard labels itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) campaign: Option<CampaignFields>,
    pub(crate) visitors: u32,
    /// The visitors from imported data, who have no revenue to convert into.
    pub(crate) imported_visitors: u32,
    /// Visitors with attributed revenue.
    pub(crate) converting_visitors: u32,
    pub(crate) revenue: i64,
}

#[derive(Debug, Default, Serialize)]
pub(crate) struct OverviewBreakdowns {
    pub(crate) sources: Vec<BreakdownRow>,
    pub(crate) channels: Vec<BreakdownRow>,
    /// The campaign rows of `sources`, ranked among themselves.
    pub(crate) campaigns: Vec<BreakdownRow>,
    pub(crate) countries: Vec<BreakdownRow>,
    pub(crate) cities: Vec<BreakdownRow>,
    pub(crate) pages: Vec<BreakdownRow>,
    pub(crate) landing_pages: Vec<BreakdownRow>,
    pub(crate) browsers: Vec<BreakdownRow>,
    pub(crate) os: Vec<BreakdownRow>,
    pub(crate) devices: Vec<BreakdownRow>,
}

/// The dimensions read off the landing row.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LandingDimension {
    Source,
    Country,
    LandingPage,
    Attribute(LandingAttribute),
}

/// [`LandingDimension::Source`]'s index in [`LANDING_DIMENSIONS`].
const SOURCE_INDEX: usize = 0;

const LANDING_DIMENSIONS: [LandingDimension; 9] = [
    LandingDimension::Source,
    LandingDimension::Country,
    LandingDimension::LandingPage,
    LandingDimension::Attribute(LandingAttribute::Channel),
    LandingDimension::Attribute(LandingAttribute::Region),
    LandingDimension::Attribute(LandingAttribute::City),
    LandingDimension::Attribute(LandingAttribute::Browser),
    LandingDimension::Attribute(LandingAttribute::Os),
    LandingDimension::Attribute(LandingAttribute::Device),
];

/// The page dimension's bit in a session's failed-filter mask; landing
/// dimensions use their index in [`LANDING_DIMENSIONS`].
const PAGE_BIT: u16 = 1 << LANDING_DIMENSIONS.len();

/// Marks a session with no value for a dimension (no resolved country, say).
const NO_VALUE: u32 = u32::MAX;

/// Distinct values of one dimension, numbered so sessions can store ids.
#[derive(Default)]
struct Interner {
    ids: FxHashMap<String, u32>,
    values: Vec<String>,
}

impl Interner {
    fn intern(&mut self, value: &str) -> u32 {
        if let Some(&id) = self.ids.get(value) {
            return id;
        }
        let id = self.values.len() as u32;
        self.ids.insert(value.to_string(), id);
        self.values.push(value.to_string());
        id
    }
}

/// The period's landings with the facts each dimension reads, in session order.
fn landings_sql(event_filter: &str) -> String {
    format!(
        "SELECT l.visitor_timestamp_id,
                l.country,
                l.region,
                l.city,
                l.browser_family,
                l.os,
                l.device,
                l.referrer,
                l.campaign,
                c.deleted_at IS NULL,
                l.referrer_type,
                lp.path_id
         FROM landings l
         JOIN pageviews lp ON lp.timestamp_id = l.visitor_timestamp_id
         LEFT JOIN campaigns c ON c.id = l.campaign
         WHERE l.visitor_timestamp_id >= ?1
           AND l.visitor_timestamp_id < ?2{event_filter}
         ORDER BY l.visitor_timestamp_id"
    )
}

/// The period's pageviews in session order. Rows of sessions the landings scan
/// leaves out (by the event filter) are skipped while merging, so this needs no
/// filter of its own.
const PAGEVIEWS_SQL: &str = "SELECT pv.visitor_timestamp_id, pv.path_id
     FROM pageviews pv
     WHERE pv.visitor_timestamp_id >= ?1
       AND pv.visitor_timestamp_id < ?2
     ORDER BY pv.visitor_timestamp_id";

/// A query's `(visitor_timestamp_id, value)` rows, read in visitor order
/// alongside the landings.
struct VisitorCursor<'stmt> {
    rows: rusqlite::Rows<'stmt>,
    next: Option<(i64, i64)>,
}

impl<'stmt> VisitorCursor<'stmt> {
    fn new(mut rows: rusqlite::Rows<'stmt>) -> rusqlite::Result<Self> {
        let next = Self::read(&mut rows)?;
        Ok(Self { rows, next })
    }

    fn read(rows: &mut rusqlite::Rows<'stmt>) -> rusqlite::Result<Option<(i64, i64)>> {
        rows.next()?
            .map(|row| Ok((row.get(0)?, row.get(1)?)))
            .transpose()
    }

    /// Calls `each` with the value of every row for `visitor`, first skipping
    /// rows of earlier visitors, which the landings scan left out.
    fn for_visitor(&mut self, visitor: i64, mut each: impl FnMut(i64)) -> rusqlite::Result<()> {
        while let Some((row_visitor, value)) = self.next
            && row_visitor <= visitor
        {
            if row_visitor == visitor {
                each(value);
            }
            self.next = Self::read(&mut self.rows)?;
        }
        Ok(())
    }
}

#[derive(Default, Clone, Copy)]
struct Tally {
    visitors: u32,
    imported_visitors: u32,
    converting_visitors: u32,
    revenue: i64,
}

impl Tally {
    /// Imported sessions have no revenue of their own to attribute.
    fn add_imported(&mut self, sessions: i64) {
        let sessions = u32::try_from(sessions).unwrap_or(u32::MAX);
        self.visitors = self.visitors.saturating_add(sessions);
        self.imported_visitors = self.imported_visitors.saturating_add(sessions);
    }

    fn add(&mut self, revenue: i64) {
        self.visitors += 1;
        if revenue > 0 {
            self.converting_visitors += 1;
        }
        self.revenue += revenue;
    }
}

/// Builds every breakdown for sessions that landed in `[start, end)` (seconds).
pub(crate) fn build_overview_breakdowns(
    conn: &ProjectConnection,
    start: i64,
    end: i64,
    segments: &SegmentFilters,
    imported: Option<&ImportedDays>,
) -> Result<OverviewBreakdowns, AuthError> {
    let start_ns = start * NANOS_PER_SECOND;
    let end_ns = end * NANOS_PER_SECOND;
    let mut interners: [Interner; LANDING_DIMENSIONS.len()] = Default::default();
    // Soft-deleted pages are hidden from the page views. They only lose their
    // rows here: the SQL page filters still match them, so filtering does too.
    let deleted_paths: FxHashSet<i64> = conn
        .with_query(
            "SELECT id FROM paths WHERE deleted_at IS NOT NULL",
            [],
            |mut rows| {
                let mut ids = FxHashSet::default();
                while let Some(row) = rows.next()? {
                    ids.insert(row.get(0)?);
                }
                Ok(ids)
            },
        )
        .map_err(map_row_error)?;

    let selected: [Option<FxHashSet<String>>; LANDING_DIMENSIONS.len()] =
        LANDING_DIMENSIONS.map(|dimension| {
            let values: FxHashSet<String> = match dimension {
                LandingDimension::Source => segments.sources.iter().map(|s| s.param()).collect(),
                LandingDimension::Country => segments.countries.iter().cloned().collect(),
                LandingDimension::LandingPage => {
                    segments.landing_pages.iter().map(i64::to_string).collect()
                }
                LandingDimension::Attribute(attribute) => segments
                    .attribute_values(attribute)
                    .map(str::to_string)
                    .collect(),
            };
            (!values.is_empty()).then_some(values)
        });
    let selected_pages: Option<FxHashSet<i64>> =
        (!segments.pages.is_empty()).then(|| segments.pages.iter().copied().collect());

    let events_only = segments.events_only();
    let event_filter = events_only.session_filter_clause("l.visitor_timestamp_id", None);
    let landings_sql = landings_sql(&event_filter);
    // Same window as the landings scan, first subscription charge only.
    let revenue_sql = page_revenue_select(1, 2, true);
    let params: [&dyn ToSql; 2] = [&start_ns, &end_ns];

    let mut tallies: [FxHashMap<u32, Tally>; LANDING_DIMENSIONS.len()] = Default::default();
    let mut page_tallies: FxHashMap<i64, Tally> = FxHashMap::default();
    // Landings, their pageviews and their revenue are each read in session
    // order and merged as they go, so each session is tallied and dropped once
    // its rows are read; memory grows with the number of distinct values, not
    // of sessions. The query plans read all three off indexes in that order,
    // with no sort; a test checks them.
    let mut scan = || -> rusqlite::Result<()> {
        // One read transaction gives the three cursors one snapshot. Without it
        // a cursor whose window is empty finishes at once, and the next starts
        // a new snapshot that may include a session committed in between.
        let _snapshot = conn.unchecked_transaction()?;
        let mut pageviews_stmt = conn.prepare_cached(PAGEVIEWS_SQL)?;
        let mut pageviews = VisitorCursor::new(pageviews_stmt.query(params_from_iter(params))?)?;
        let mut revenue_stmt = conn.prepare_cached(&revenue_sql)?;
        let mut paid = VisitorCursor::new(revenue_stmt.query(params_from_iter(params))?)?;
        let mut landings_stmt = conn.prepare_cached(&landings_sql)?;
        let mut rows = landings_stmt.query(params_from_iter(params))?;
        let mut value = String::new();
        let mut pages: SmallVec<[i64; 8]> = SmallVec::new();
        while let Some(row) = rows.next()? {
            let visitor_timestamp_id: i64 = row.get(0)?;
            let country: Option<SqlCompactStr> = row.get(1)?;
            let region: Option<SqlCompactStr> = row.get(2)?;
            let city: Option<SqlCompactStr> = row.get(3)?;
            let browser_family: Option<SqlCompactStr> = row.get(4)?;
            let os: Option<SqlCompactStr> = row.get(5)?;
            let device: SqlCompactStr = row.get(6)?;
            let referrer: Option<i64> = row.get(7)?;
            let campaign: Option<i64> = row.get(8)?;
            let campaign_is_live: bool = row.get(9)?;
            let referrer_type: Option<SqlCompactStr> = row.get(10)?;
            let landing_path: i64 = row.get(11)?;
            let live_campaign = campaign.filter(|_| campaign_is_live);
            let facts = LandingFacts {
                country: country.as_ref().map(SqlCompactStr::as_str),
                region: region.as_ref().map(SqlCompactStr::as_str),
                city: city.as_ref().map(SqlCompactStr::as_str),
                browser_family: browser_family.as_ref().map(SqlCompactStr::as_str),
                os: os.as_ref().map(SqlCompactStr::as_str),
                device: device.as_str(),
                has_referrer: referrer.is_some(),
                has_live_campaign: live_campaign.is_some(),
                referrer_type: referrer_type.as_ref().map(SqlCompactStr::as_str),
            };
            // The SQL source filter still matches a soft-deleted campaign the
            // session landed from, though its row shows the referrer.
            let deleted_campaign_source = match campaign {
                Some(id) if !campaign_is_live => Some(SourceFilter::Campaign(id).param()),
                _ => None,
            };

            let mut values = [NO_VALUE; LANDING_DIMENSIONS.len()];
            let mut failed: u16 = 0;
            for (index, dimension) in LANDING_DIMENSIONS.iter().enumerate() {
                value.clear();
                let has_value = match dimension {
                    LandingDimension::Source => {
                        let source = SourceFilter::for_landing(live_campaign, referrer);
                        write!(value, "{source}").is_ok()
                    }
                    LandingDimension::Country => facts.country.is_some_and(|country| {
                        value.push_str(country);
                        true
                    }),
                    LandingDimension::LandingPage => write!(value, "{landing_path}").is_ok(),
                    // A value the filter parser would reject could be shown
                    // but never applied, so it gets no row.
                    LandingDimension::Attribute(attribute) => {
                        attribute.write_value(&facts, &mut value) && attribute.accepts(&value)
                    }
                };
                if let Some(selected) = &selected[index] {
                    let matches = (has_value && selected.contains(value.as_str()))
                        || (index == SOURCE_INDEX
                            && deleted_campaign_source
                                .as_ref()
                                .is_some_and(|source| selected.contains(source)));
                    if !matches {
                        failed |= 1 << index;
                    }
                }
                if has_value {
                    values[index] = interners[index].intern(&value);
                }
            }

            pages.clear();
            pageviews.for_visitor(visitor_timestamp_id, |path| {
                if !pages.contains(&path) {
                    pages.push(path);
                }
            })?;
            let mut revenue = 0;
            paid.for_visitor(visitor_timestamp_id, |amount| revenue += amount)?;
            if let Some(selected_pages) = &selected_pages
                && !pages.iter().any(|page| selected_pages.contains(page))
            {
                failed |= PAGE_BIT;
            }

            // A session counts toward a breakdown when the only filter it
            // fails, if any, is that breakdown's own.
            if failed.count_ones() > 1 {
                continue;
            }
            for (index, tally) in tallies.iter_mut().enumerate() {
                let value = values[index];
                if value != NO_VALUE && (failed == 0 || failed == 1 << index) {
                    tally.entry(value).or_default().add(revenue);
                }
            }
            if failed == 0 || failed == PAGE_BIT {
                for page in pages.iter().filter(|page| !deleted_paths.contains(page)) {
                    page_tallies.entry(*page).or_default().add(revenue);
                }
            }
        }
        Ok(())
    };
    scan().map_err(map_row_error)?;
    if let Some(days) = imported {
        add_imported_tallies(conn, days, &mut interners, &mut tallies, &mut page_tallies)?;
    }

    // Ranked apart from the other sources so campaigns outside the overall top
    // still get a row.
    let source_values = &interners[SOURCE_INDEX].values;
    let top_campaigns = top_rows(
        tallies[SOURCE_INDEX]
            .iter()
            .map(|(id, tally)| (source_values[*id as usize].clone(), *tally))
            .filter(|(value, _)| value.starts_with("campaign-")),
        BREAKDOWN_ROWS,
    );
    let mut top: Vec<Vec<(String, Tally)>> = tallies
        .into_iter()
        .zip(&interners)
        .zip(LANDING_DIMENSIONS)
        .map(|((tally, interner), dimension)| {
            top_rows(
                tally
                    .into_iter()
                    .map(|(id, tally)| (interner.values[id as usize].clone(), tally))
                    .filter(|(value, _)| {
                        dimension != LandingDimension::LandingPage
                            || value
                                .parse()
                                .is_ok_and(|path| !deleted_paths.contains(&path))
                    }),
                BREAKDOWN_ROWS,
            )
        })
        .collect();
    let top_pages = top_rows(
        page_tallies
            .into_iter()
            .map(|(page, tally)| (page.to_string(), tally)),
        BREAKDOWN_ROWS,
    );

    let index_of = |dimension: LandingDimension| {
        LANDING_DIMENSIONS
            .iter()
            .position(|candidate| *candidate == dimension)
            .expect("every dimension is listed")
    };
    let mut take = |dimension| std::mem::take(&mut top[index_of(dimension)]);
    let sources = take(LandingDimension::Source);
    let countries = take(LandingDimension::Country);
    let landing_pages = take(LandingDimension::LandingPage);
    let channels = take(LandingDimension::Attribute(LandingAttribute::Channel));
    let cities = take(LandingDimension::Attribute(LandingAttribute::City));
    let browsers = take(LandingDimension::Attribute(LandingAttribute::Browser));
    let os = take(LandingDimension::Attribute(LandingAttribute::Os));
    let devices = take(LandingDimension::Attribute(LandingAttribute::Device));

    let path_ids: Vec<i64> = landing_pages
        .iter()
        .chain(&top_pages)
        .filter_map(|(value, _)| value.parse().ok())
        .collect::<FxHashSet<i64>>()
        .into_iter()
        .collect();
    let paths: FxHashMap<i64, (String, String)> = fetch_by_ids(
        conn,
        "SELECT id, hostname, path FROM paths WHERE id IN",
        &path_ids,
        |row| Ok((row.get(0)?, (row.get(1)?, row.get(2)?))),
    )?;
    // Decided across both page tabs so a page reads the same in each.
    let with_hostname = paths
        .values()
        .map(|(hostname, _)| hostname.as_str())
        .collect::<FxHashSet<&str>>()
        .len()
        > 1;

    Ok(OverviewBreakdowns {
        sources: source_rows(conn, sources)?,
        campaigns: source_rows(conn, top_campaigns)?,
        channels: labeled(channels, |value| {
            SourceGroup::parse(value)
                .map(SourceGroup::label)
                .unwrap_or(value)
                .to_string()
        }),
        countries: labeled(countries, |value| country_name_for_country(Some(value))),
        cities: labeled(cities, |value| after_country(value).to_string()),
        pages: path_rows(top_pages, &paths, with_hostname),
        landing_pages: path_rows(landing_pages, &paths, with_hostname),
        browsers: labeled(browsers, browser_family_name),
        os: labeled(os, os_name),
        devices: labeled(devices, device_name),
    })
}

/// Counts imported sessions under the same values tracked sessions have, so each
/// row is one source, place or page whichever way its sessions were counted.
fn add_imported_tallies(
    conn: &ProjectConnection,
    days: &ImportedDays,
    interners: &mut [Interner; LANDING_DIMENSIONS.len()],
    tallies: &mut [FxHashMap<u32, Tally>; LANDING_DIMENSIONS.len()],
    page_tallies: &mut FxHashMap<i64, Tally>,
) -> Result<(), AuthError> {
    let mut add = |dimension: LandingDimension, value: &str, sessions: i64| {
        let index = LANDING_DIMENSIONS
            .iter()
            .position(|candidate| *candidate == dimension)
            .expect("every dimension is listed");
        let id = interners[index].intern(value);
        tallies[index].entry(id).or_default().add_imported(sessions);
    };
    for source in days.sources(conn)? {
        if let Some(value) = &source.source {
            add(LandingDimension::Source, value, source.sessions);
        }
        add(
            LandingDimension::Attribute(LandingAttribute::Channel),
            &source.channel,
            source.sessions,
        );
    }
    for place in days.places(conn)? {
        add(LandingDimension::Country, &place.country, place.sessions);
        if let Some(city) = &place.city {
            let value = format!("{}-{city}", place.country);
            if LandingAttribute::City.accepts(&value) {
                add(
                    LandingDimension::Attribute(LandingAttribute::City),
                    &value,
                    place.sessions,
                );
            }
        }
    }
    for row in days.technology(conn)? {
        for (attribute, value) in [
            (LandingAttribute::Browser, &row.browser_family),
            (LandingAttribute::Os, &row.os),
            (LandingAttribute::Device, &row.device),
        ] {
            add(LandingDimension::Attribute(attribute), value, row.sessions);
        }
    }
    for page in days.landing_pages(conn)? {
        add(
            LandingDimension::LandingPage,
            &page.path_id.to_string(),
            page.sessions,
        );
    }
    for page in days.pages(conn)? {
        page_tallies
            .entry(page.path_id)
            .or_default()
            .add_imported(page.sessions);
    }
    Ok(())
}

/// The top `limit` by visitors plus any others in the top `limit` by revenue,
/// ordered by visitors.
fn top_rows(tallies: impl Iterator<Item = (String, Tally)>, limit: usize) -> Vec<(String, Tally)> {
    let mut rows: Vec<(String, Tally)> = tallies.collect();
    rows.sort_by(|(a_value, a), (b_value, b)| {
        b.revenue
            .cmp(&a.revenue)
            .then(b.visitors.cmp(&a.visitors))
            .then_with(|| a_value.cmp(b_value))
    });
    let top_revenue: Vec<String> = rows
        .iter()
        .take(limit)
        .filter(|(_, tally)| tally.revenue > 0)
        .map(|(value, _)| value.clone())
        .collect();
    rows.sort_by(|(a_value, a), (b_value, b)| {
        b.visitors
            .cmp(&a.visitors)
            .then(b.revenue.cmp(&a.revenue))
            .then_with(|| a_value.cmp(b_value))
    });
    let mut kept = 0;
    rows.retain(|(value, _)| {
        kept += 1;
        kept <= limit || top_revenue.contains(value)
    });
    rows
}

fn labeled(rows: Vec<(String, Tally)>, label: impl Fn(&str) -> String) -> Vec<BreakdownRow> {
    rows.into_iter()
        .map(|(value, tally)| BreakdownRow {
            label: label(&value),
            value,
            campaign: None,
            visitors: tally.visitors,
            imported_visitors: tally.imported_visitors,
            converting_visitors: tally.converting_visitors,
            revenue: tally.revenue,
        })
        .collect()
}

/// Labels page rows with their path, or with hostname and path when the
/// panels span hostnames, since the same path on two sites is two pages.
fn path_rows(
    rows: Vec<(String, Tally)>,
    paths: &FxHashMap<i64, (String, String)>,
    with_hostname: bool,
) -> Vec<BreakdownRow> {
    labeled(rows, |value| {
        match value.parse::<i64>().ok().and_then(|id| paths.get(&id)) {
            Some((hostname, path)) if with_hostname => format!("{hostname}{path}"),
            Some((_, path)) => path.clone(),
            None => value.to_string(),
        }
    })
}

/// `Springfield` from a `US-Springfield` city value.
fn after_country(value: &str) -> &str {
    value.split_once('-').map_or(value, |(_, rest)| rest)
}

fn source_rows(
    conn: &ProjectConnection,
    rows: Vec<(String, Tally)>,
) -> Result<Vec<BreakdownRow>, AuthError> {
    let ids_with_prefix = |prefix: &str| -> Vec<i64> {
        rows.iter()
            .filter_map(|(value, _)| value.strip_prefix(prefix)?.parse().ok())
            .collect()
    };
    let referrers: FxHashMap<i64, String> = fetch_by_ids(
        conn,
        "SELECT id, referrer FROM referrers WHERE id IN",
        &ids_with_prefix("referrer-"),
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let mut campaigns: FxHashMap<i64, CampaignFields> = fetch_by_ids(
        conn,
        "SELECT id, source, medium, campaign, network, display_name FROM campaigns WHERE id IN",
        &ids_with_prefix("campaign-"),
        |row| {
            Ok((
                row.get(0)?,
                CampaignFields {
                    source: row.get(1)?,
                    medium: row.get(2)?,
                    campaign: row.get(3)?,
                    network: row.get(4)?,
                    display_name: row.get(5)?,
                },
            ))
        },
    )?;
    Ok(rows
        .into_iter()
        .map(|(value, tally)| {
            let campaign = value
                .strip_prefix("campaign-")
                .and_then(|id| campaigns.remove(&id.parse().ok()?));
            let label = if let Some(campaign) = &campaign {
                campaign
                    .display_name
                    .clone()
                    .unwrap_or_else(|| campaign.campaign.clone())
            } else if let Some(id) = value.strip_prefix("referrer-") {
                id.parse()
                    .ok()
                    .and_then(|id| referrers.get(&id).cloned())
                    .unwrap_or_else(|| value.clone())
            } else {
                "Direct".to_string()
            };
            BreakdownRow {
                value,
                label,
                campaign,
                visitors: tally.visitors,
                imported_visitors: tally.imported_visitors,
                converting_visitors: tally.converting_visitors,
                revenue: tally.revenue,
            }
        })
        .collect())
}

/// Runs `select_where_id_in (?1, ...)` over `ids`, collecting what `read` returns per row.
fn fetch_by_ids<T>(
    conn: &ProjectConnection,
    select_where_id_in: &str,
    ids: &[i64],
    read: impl Fn(&rusqlite::Row<'_>) -> rusqlite::Result<(i64, T)>,
) -> Result<FxHashMap<i64, T>, AuthError> {
    if ids.is_empty() {
        return Ok(FxHashMap::default());
    }
    let placeholders = (1..=ids.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    conn.with_query(
        &format!("{select_where_id_in} ({placeholders})"),
        params_from_iter(ids),
        |mut rows| {
            let mut found = FxHashMap::default();
            while let Some(row) = rows.next()? {
                let (id, value) = read(row)?;
                found.insert(id, value);
            }
            Ok(found)
        },
    )
    .map_err(map_row_error)
}

#[cfg(test)]
mod tests {

    use super::*;

    use crate::segment_filter::parse_segment_filters;
    use crate::test_utils::{
        LandingFixture, PaymentFixture, insert_landing, insert_pageview, insert_payment,
    };

    const START: i64 = 1_000;
    const END: i64 = 2_000;

    fn session_id(second: i64, seed: i64) -> i64 {
        second * NANOS_PER_SECOND + seed
    }

    /// Three sessions in the window and one before it:
    /// - A: US/NY, chrome/macos/desktop, from google.com (search), lands on `/`,
    ///   views `/pricing`, pays 5000.
    /// - B: US/CA, safari/ios/mobile, direct, lands on `/pricing`, pays nothing.
    /// - C: FR, chrome/windows/desktop, from the live campaign, lands on `/blog`,
    ///   views `/pricing`, pays 2000.
    fn seeded() -> (ProjectConnection, [i64; 3]) {
        let conn = crate::test_utils::open_migrated_db();
        conn.execute_batch(
            "INSERT INTO paths (id, path, hostname) VALUES
                 (1, '/', 'acme.test'), (2, '/pricing', 'acme.test'), (3, '/blog', 'acme.test');
             INSERT INTO referrers (id, referrer) VALUES (1, 'google.com');
             INSERT INTO campaigns (id, source, medium, campaign, display_name, first_seen_at)
                 VALUES (1, 'newsletter', 'email', 'launch', 'Launch', 0);",
        )
        .expect("seed");
        let a = session_id(1_100, 1);
        let b = session_id(1_200, 2);
        let c = session_id(1_300, 3);
        let before = session_id(900, 4);
        insert_landing(
            &conn,
            LandingFixture {
                visitor_timestamp_id: a,
                path_id: 1,
                country: Some("US"),
                region: Some("NY"),
                city: Some("New York"),
                browser_family: Some("chrome"),
                os: Some("macos"),
                referrer: Some(1),
                referrer_type: Some("search"),
                ..LandingFixture::default()
            },
        );
        insert_pageview(&conn, a + 10, a, 2);
        insert_landing(
            &conn,
            LandingFixture {
                visitor_timestamp_id: b,
                path_id: 2,
                country: Some("US"),
                region: Some("CA"),
                browser_family: Some("safari"),
                os: Some("ios"),
                device: Some("mobile"),
                ..LandingFixture::default()
            },
        );
        insert_landing(
            &conn,
            LandingFixture {
                visitor_timestamp_id: c,
                path_id: 3,
                country: Some("FR"),
                browser_family: Some("chrome"),
                os: Some("windows"),
                campaign: Some(1),
                ..LandingFixture::default()
            },
        );
        insert_pageview(&conn, c + 10, c, 2);
        insert_landing(
            &conn,
            LandingFixture {
                visitor_timestamp_id: before,
                path_id: 1,
                country: Some("DE"),
                ..LandingFixture::default()
            },
        );
        for (id, session, amount) in [("a", a, 5_000), ("c", c, 2_000), ("before", before, 900)] {
            let mut payment = PaymentFixture::new(format!("pay_{id}"), amount, 1_500);
            payment.visitor_timestamp_id = Some(session);
            insert_payment(&conn, payment);
        }
        (conn, [a, b, c])
    }

    fn breakdowns(conn: &ProjectConnection, query: &str) -> OverviewBreakdowns {
        build_overview_breakdowns(conn, START, END, &parse_segment_filters(query), None)
            .expect("breakdowns")
    }

    /// `(value, visitors, revenue)` per row, in response order.
    fn summary(rows: &[BreakdownRow]) -> Vec<(&str, u32, i64)> {
        rows.iter()
            .map(|row| (row.value.as_str(), row.visitors, row.revenue))
            .collect()
    }

    #[test]
    fn counts_visitors_and_attributed_revenue_per_dimension() {
        let (conn, _) = seeded();
        let result = breakdowns(&conn, "");

        assert_eq!(
            summary(&result.countries),
            vec![("US", 2, 5_000), ("FR", 1, 2_000)]
        );
        assert_eq!(result.countries[0].label, "United States");
        assert_eq!(
            summary(&result.sources),
            vec![
                ("referrer-1", 1, 5_000),
                ("campaign-1", 1, 2_000),
                ("direct", 1, 0)
            ]
        );
        assert_eq!(result.sources[0].label, "google.com");
        assert_eq!(result.sources[1].label, "Launch");
        assert_eq!(
            result.sources[1]
                .campaign
                .as_ref()
                .map(|c| c.source.as_str()),
            Some("newsletter")
        );
        assert_eq!(result.sources[2].label, "Direct");
        assert_eq!(
            summary(&result.channels),
            vec![
                ("search", 1, 5_000),
                ("campaigns", 1, 2_000),
                ("direct", 1, 0)
            ]
        );
        assert_eq!(summary(&result.campaigns), vec![("campaign-1", 1, 2_000)]);
        assert_eq!(result.campaigns[0].label, "Launch");
        assert_eq!(summary(&result.cities), vec![("US-New York", 1, 5_000)]);
        assert_eq!(
            summary(&result.pages),
            vec![("2", 3, 7_000), ("1", 1, 5_000), ("3", 1, 2_000)]
        );
        assert_eq!(result.pages[0].label, "/pricing");
        assert_eq!(
            summary(&result.landing_pages),
            vec![("1", 1, 5_000), ("3", 1, 2_000), ("2", 1, 0)]
        );
        assert_eq!(
            summary(&result.browsers),
            vec![("chrome", 2, 7_000), ("safari", 1, 0)]
        );
        assert_eq!(result.browsers[0].label, "Chrome");
        assert_eq!(result.os[0].label, "macOS");
        assert_eq!(
            summary(&result.devices),
            vec![("desktop", 2, 7_000), ("mobile", 1, 0)]
        );
        assert_eq!(result.devices[0].converting_visitors, 2);
    }

    #[test]
    fn a_filter_narrows_every_breakdown_but_its_own() {
        let (conn, _) = seeded();
        let result = breakdowns(&conn, "?country=US");
        assert_eq!(
            summary(&result.countries),
            vec![("US", 2, 5_000), ("FR", 1, 2_000)]
        );
        assert_eq!(
            summary(&result.browsers),
            vec![("chrome", 1, 5_000), ("safari", 1, 0)]
        );
        assert_eq!(
            summary(&result.pages),
            vec![("2", 2, 5_000), ("1", 1, 5_000)]
        );

        let result = breakdowns(&conn, "?page=3");
        assert_eq!(
            summary(&result.pages),
            vec![("2", 3, 7_000), ("1", 1, 5_000), ("3", 1, 2_000)]
        );
        assert_eq!(summary(&result.countries), vec![("FR", 1, 2_000)]);
    }

    #[test]
    fn a_session_failing_two_filters_counts_nowhere() {
        let (conn, _) = seeded();
        // A and B fail the page filter; C fails the country filter.
        let result = breakdowns(&conn, "?country=US&page=3");
        assert_eq!(summary(&result.countries), vec![("FR", 1, 2_000)]);
        assert_eq!(
            summary(&result.pages),
            vec![("2", 2, 5_000), ("1", 1, 5_000)]
        );
        assert!(result.browsers.is_empty());
    }

    #[test]
    fn landing_attribute_and_source_filters_apply() {
        let (conn, _) = seeded();
        let result = breakdowns(&conn, "?device=mobile");
        assert_eq!(
            summary(&result.devices),
            vec![("desktop", 2, 7_000), ("mobile", 1, 0)]
        );
        assert_eq!(summary(&result.countries), vec![("US", 1, 0)]);

        let result = breakdowns(&conn, "?source=campaign-1&channel=campaigns");
        assert_eq!(summary(&result.countries), vec![("FR", 1, 2_000)]);
        assert_eq!(result.sources.len(), 1);
        assert_eq!(result.channels.len(), 1);

        let result = breakdowns(&conn, "?landing_page=2");
        assert_eq!(summary(&result.os), vec![("ios", 1, 0)]);
        assert_eq!(result.landing_pages.len(), 3);
    }

    #[test]
    fn event_filter_narrows_every_breakdown() {
        let (conn, [_, _, c]) = seeded();
        conn.execute_batch("INSERT INTO event_names (id, name) VALUES (1, 'signup');")
            .expect("event name");
        conn.execute(
            "INSERT INTO events (timestamp_id, event_name_id, page_timestamp_id) VALUES (?1, 1, ?2)",
            rusqlite::params![c + 20, c + 10],
        )
        .expect("event");
        let result = breakdowns(&conn, "?event=1");
        assert_eq!(summary(&result.countries), vec![("FR", 1, 2_000)]);
        assert_eq!(
            summary(&result.pages),
            vec![("2", 1, 2_000), ("3", 1, 2_000)]
        );
    }

    #[test]
    fn empty_period_has_no_rows() {
        let (conn, _) = seeded();
        let result =
            build_overview_breakdowns(&conn, 5_000, 6_000, &SegmentFilters::default(), None)
                .unwrap();
        assert!(result.countries.is_empty() && result.pages.is_empty());
    }

    /// The merge relies on each scan arriving in session order without a sort,
    /// which would hold the whole period in memory.
    #[test]
    fn scans_stream_in_session_order_without_sorting() {
        let conn = crate::test_utils::open_migrated_db();
        let event_filter = parse_segment_filters("?event=1")
            .events_only()
            .session_filter_clause("l.visitor_timestamp_id", None);
        for sql in [
            landings_sql(""),
            landings_sql(&event_filter),
            PAGEVIEWS_SQL.to_string(),
            page_revenue_select(1, 2, true),
        ] {
            let plan: Vec<String> = conn
                .with_query(
                    &format!("EXPLAIN QUERY PLAN {sql}"),
                    [0_i64, 1],
                    |mut rows| {
                        let mut plan = Vec::new();
                        while let Some(row) = rows.next()? {
                            plan.push(row.get(3)?);
                        }
                        Ok(plan)
                    },
                )
                .expect("plan");
            assert!(
                plan.iter().all(|step| !step.contains("TEMP B-TREE")),
                "{sql}\nsorts: {plan:?}"
            );
        }
    }

    #[test]
    fn keeps_top_rows_by_visitors_and_by_revenue() {
        let tally = |visitors, revenue| Tally {
            visitors,
            imported_visitors: 0,
            converting_visitors: 0,
            revenue,
        };
        let mut tallies: Vec<(String, Tally)> = (0..15)
            .map(|index| (format!("busy-{index:02}"), tally(100 - index, 0)))
            .collect();
        tallies.push(("rich".to_string(), tally(1, 9_000)));
        let rows = top_rows(tallies.into_iter(), 10);
        let values: Vec<&str> = rows.iter().map(|(value, _)| value.as_str()).collect();
        assert_eq!(values.len(), 11);
        assert_eq!(values[0], "busy-00");
        assert_eq!(values[9], "busy-09");
        assert_eq!(values[10], "rich");
    }

    /// Breakdown rows must cover the same sessions the production SQL filter
    /// selects for every other route, or the panels would disagree with the
    /// chart above them.
    #[test]
    fn breakdown_totals_match_the_sql_segment_filter() {
        let (conn, [_, b, _]) = seeded();
        // B now lands from a campaign that was deleted after the fact.
        conn.execute_batch(
            "INSERT INTO campaigns (id, campaign, first_seen_at, deleted_at)
                 VALUES (2, 'gone', 0, 1);",
        )
        .expect("deleted campaign");
        conn.execute(
            "UPDATE landings SET campaign = 2 WHERE visitor_timestamp_id = ?1",
            [b],
        )
        .expect("assign deleted campaign");

        for query in [
            "",
            "?source=campaign-2",
            "?source=direct",
            "?source=campaign-1&source=referrer-1",
            "?country=US",
            "?landing_page=2&landing_page=3",
            "?page=2",
            "?channel=direct",
            "?channel=campaigns",
            "?region=US-CA",
            "?city=US-New+York",
            "?browser=chrome&os=windows",
            "?device=desktop&country=FR",
        ] {
            let segments = parse_segment_filters(query);
            let clause = segments.session_filter_clause("l.visitor_timestamp_id", None);
            let country = segments.country_in_clause("l.country", 3);
            let start_ns = START * NANOS_PER_SECOND;
            let end_ns = END * NANOS_PER_SECOND;
            let mut params: Vec<&dyn ToSql> = vec![&start_ns, &end_ns];
            segments.append_country_params(&mut params);
            let expected: u32 = conn
                .query_row(
                    &format!(
                        "SELECT COUNT(*) FROM landings l
                         WHERE l.visitor_timestamp_id >= ?1
                           AND l.visitor_timestamp_id < ?2{country}{clause}"
                    ),
                    params_from_iter(params),
                    |row| row.get(0),
                )
                .expect("sql count");
            // A panel exempt from none of the query's filters sums to the
            // sessions every filter selects.
            let result = breakdowns(&conn, query);
            let panel = if query.contains("device=") {
                &result.browsers
            } else {
                &result.devices
            };
            let total: u32 = panel.iter().map(|row| row.visitors).sum();
            assert_eq!(total, expected, "{query}");
        }
    }

    #[test]
    fn deleted_pages_lose_their_rows_but_still_filter() {
        let (conn, _) = seeded();
        conn.execute("UPDATE paths SET deleted_at = 1 WHERE id = 2", [])
            .expect("delete page");
        let result = breakdowns(&conn, "");
        assert_eq!(
            summary(&result.pages),
            vec![("1", 1, 5_000), ("3", 1, 2_000)]
        );
        assert_eq!(
            summary(&result.landing_pages),
            vec![("1", 1, 5_000), ("3", 1, 2_000)]
        );
        let result = breakdowns(&conn, "?landing_page=2");
        assert_eq!(summary(&result.os), vec![("ios", 1, 0)]);
    }

    #[test]
    fn page_labels_name_the_hostname_only_when_rows_span_hostnames() {
        let (conn, _) = seeded();
        assert_eq!(breakdowns(&conn, "").pages[0].label, "/pricing");
        conn.execute(
            "UPDATE paths SET hostname = 'blog.acme.test' WHERE id = 3",
            [],
        )
        .expect("second hostname");
        let labels: Vec<String> = breakdowns(&conn, "")
            .pages
            .into_iter()
            .map(|row| row.label)
            .collect();
        assert_eq!(
            labels,
            vec!["acme.test/pricing", "acme.test/", "blog.acme.test/blog"]
        );
    }
}
