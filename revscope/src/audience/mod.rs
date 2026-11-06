//! The audience report: visitors and revenue by country, city, browser, OS,
//! device and screen and viewport width, each against the prior 30 days.

mod rows;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use rusqlite::params_from_iter;
use rusqlite::types::ToSql;
use serde::Serialize;

use crate::auth::{AuthError, map_row_error};
use crate::authorize::AuthorizedProject;
use crate::comparison;
use crate::db::{ProjectConnection, open_project_database_read};
use crate::ga_import::{ImportedDays, ImportedWindows};
use crate::page_revenue::page_revenue_cte;
use crate::query_params::{SortDirection, SortedPage, parse_report_query};
use crate::segment_filter::{LandingAttribute, SegmentFilters, parse_segment_filters};
use crate::server_context::ServerContext;
use crate::timezone;
use crate::util::{NANOS_PER_SECOND, seconds_since_epoch};

use rows::{
    BrowsersData, CitiesData, CitiesOrderBy, CityKey, CountriesData, DevicesData, DimensionCounts,
    OsData, ScreenSizesData, ViewportsData, browser_rows, city_rows, country_rows, device_rows,
    os_rows, screen_size_rows, viewport_rows,
};

/// Landing attribute filters applied per row rather than in SQL, because a
/// breakdown here shows each of them.
const ROW_EVALUATED_ATTRIBUTES: [LandingAttribute; 4] = [
    LandingAttribute::City,
    LandingAttribute::Browser,
    LandingAttribute::Os,
    LandingAttribute::Device,
];

/// Audience response. Dimension data lives under named keys so more audience
/// breakdowns can be added as siblings later.
#[derive(Debug, Serialize)]
pub(crate) struct AudienceResponse {
    pub(crate) countries: CountriesData,
    pub(crate) cities: CitiesData,
    pub(crate) browsers: BrowsersData,
    pub(crate) os: OsData,
    pub(crate) devices: DevicesData,
    pub(crate) screen_sizes: ScreenSizesData,
    pub(crate) viewports: ViewportsData,
}

pub(crate) struct AudienceQuery {
    pub(crate) start: i64,
    pub(crate) end: i64,
    /// Applied only to the cities breakdown; other dimensions are small enums.
    pub(crate) offset: i64,
    pub(crate) order_by: CitiesOrderBy,
    pub(crate) direction: SortDirection,
    pub(crate) segments: SegmentFilters,
}

pub(crate) fn parse_audience_query(target: &str) -> Result<AudienceQuery, &'static str> {
    let mut sort = SortedPage::new(CitiesOrderBy::Revenue, CitiesOrderBy::parse);
    let range = parse_report_query(target, |key, value| sort.accept(key, value).map(drop))?;
    let (start, end) = range.validate()?;
    Ok(AudienceQuery {
        start,
        end,
        offset: sort.offset,
        order_by: sort.order_by,
        direction: sort.direction,
        segments: parse_segment_filters(target),
    })
}

pub(crate) fn audience(
    server_context: Arc<ServerContext>,
    project: &AuthorizedProject,
    query: AudienceQuery,
) -> Result<AudienceResponse, AuthError> {
    let timezone = timezone::project_timezone(server_context.clone(), project.key());
    let conn = open_project_database_read(server_context.as_ref(), project.id())?;
    let imported = ImportedWindows::for_report(
        &conn,
        &timezone,
        comparison::previous_period_start(query.start),
        query.start,
        query.end,
        &query.segments,
    )?;
    build_audience_response_at(&conn, &query, seconds_since_epoch() as i64, &imported)
}

/// Builds the audience response from an open project database connection.
///
/// Each row carries display-ready derived metrics (revenue per visitor and
/// period-over-period change ratios). Country rows additionally include
/// conversion rate and Wilson confidence interval. Changes compare against the
/// prior 30 days, with additive quantities normalized to the current period
/// length.
#[cfg(test)]
pub(crate) fn build_audience_response(
    conn: &ProjectConnection,
    query: &AudienceQuery,
) -> Result<AudienceResponse, AuthError> {
    build_audience_response_at(
        conn,
        query,
        seconds_since_epoch() as i64,
        &ImportedWindows::default(),
    )
}

/// Landing counts for every breakdown, in one window.
#[derive(Default)]
struct WindowCounts {
    countries: DimensionCounts<Option<String>>,
    cities: DimensionCounts<CityKey>,
    browsers: DimensionCounts<String>,
    os: DimensionCounts<String>,
    devices: DimensionCounts<String>,
    screen_sizes: DimensionCounts<String>,
    viewports: DimensionCounts<String>,
}

/// Bits of the per-row filters a landing fails, one per breakdown that is
/// filtered by its own dimension.
#[derive(Clone, Copy)]
#[repr(u8)]
enum RowFilter {
    Country = 1,
    City = 1 << 1,
    Browser = 1 << 2,
    Os = 1 << 3,
    Device = 1 << 4,
}

/// Inner builder with an explicit `now` so the normalization factor is
/// deterministic in tests.
fn build_audience_response_at(
    conn: &ProjectConnection,
    query: &AudienceQuery,
    now: i64,
    imported: &ImportedWindows,
) -> Result<AudienceResponse, AuthError> {
    let previous_start = comparison::previous_period_start(query.start);
    let factor = comparison::normalization_factor(query.start, query.end, now);

    // Single scan over the current period plus the prior-comparison window,
    // joined with the revenue CTE once. We fetch the raw per-landing fields and
    // bucket each row in code, avoiding repeated scans of `landings` and the
    // `page_revenue` aggregate.
    //
    // The country breakdown itself intentionally ignores country segment
    // filters so users can add another country from the audience country tab.
    // The city, browser, OS and device filters are likewise evaluated per row
    // below, so each of those breakdowns ignores only its own. Other
    // session-scoped filters (e.g. source) narrow every breakdown, including
    // the country one, so they are applied in SQL here.
    let session_filter = query
        .segments
        .without_attributes(&ROW_EVALUATED_ATTRIBUTES)
        .session_filter_clause("l.visitor_timestamp_id", None);
    // Same visit window as the landings filter (?1/?2); first subscription charge only.
    let page_revenue = page_revenue_cte(1, 2, true);
    let sql = format!(
        "WITH {page_revenue}
         SELECT
            l.country AS country,
            l.city AS city,
            COALESCE(l.browser_family, 'unknown') AS browser_family,
            COALESCE(l.os, 'unknown') AS os,
            l.device AS device,
            l.screen_width AS screen_width,
            l.viewport_width AS viewport_width,
            l.visitor_timestamp_id / ?3 AS viewed_at,
            COALESCE(rev.amount, 0) AS revenue
         FROM landings l
         LEFT JOIN page_revenue rev ON rev.visitor_timestamp_id = l.visitor_timestamp_id
         WHERE l.visitor_timestamp_id >= ?1
           AND l.visitor_timestamp_id < ?2{session_filter}"
    );

    let mut current = WindowCounts::default();
    let mut previous = WindowCounts::default();

    let previous_start_ns = previous_start * NANOS_PER_SECOND;
    let end_ns = query.end * NANOS_PER_SECOND;
    let nanos = NANOS_PER_SECOND;
    let sql_params: Vec<&dyn ToSql> = vec![&previous_start_ns, &end_ns, &nanos];

    conn.with_query(&sql, params_from_iter(sql_params), |mut rows| {
        while let Some(row) = rows.next()? {
            let country = row.get::<_, Option<String>>(0)?;
            let city = row.get::<_, Option<String>>(1)?;
            let browser_family = row.get::<_, String>(2)?;
            let os = row.get::<_, String>(3)?;
            let device = row.get::<_, String>(4)?;
            let screen_width = row.get::<_, i64>(5)?;
            let viewport_width = row.get::<_, i64>(6)?;
            let viewed_at = row.get::<_, i64>(7)?;
            let revenue = row.get::<_, i64>(8)?;
            let city_value = country
                .as_deref()
                .zip(city.as_deref())
                .map(|(country, city)| format!("{country}-{city}"));
            let segments = &query.segments;
            let mut failed = 0u8;
            for (filter, passes) in [
                (
                    RowFilter::Country,
                    country_matches_segment(country.as_deref(), segments),
                ),
                (
                    RowFilter::City,
                    segments.attribute_matches(LandingAttribute::City, city_value.as_deref()),
                ),
                (
                    RowFilter::Browser,
                    segments.attribute_matches(LandingAttribute::Browser, Some(&browser_family)),
                ),
                (
                    RowFilter::Os,
                    segments.attribute_matches(LandingAttribute::Os, Some(&os)),
                ),
                (
                    RowFilter::Device,
                    segments.attribute_matches(LandingAttribute::Device, Some(&device)),
                ),
            ] {
                if !passes {
                    failed |= filter as u8;
                }
            }
            // A breakdown counts the row when the only filter it fails is its own.
            let counts_for = |own: RowFilter| failed & !(own as u8) == 0;

            let window = if viewed_at >= query.start {
                &mut current
            } else {
                &mut previous
            };
            if counts_for(RowFilter::Country) {
                window
                    .countries
                    .entry(country.clone())
                    .or_default()
                    .add(revenue);
            }
            if counts_for(RowFilter::City)
                && let Some(city) = city
            {
                window
                    .cities
                    .entry(CityKey { country, city })
                    .or_default()
                    .add(revenue);
            }
            if counts_for(RowFilter::Browser) {
                window
                    .browsers
                    .entry(browser_family)
                    .or_default()
                    .add(revenue);
            }
            if counts_for(RowFilter::Os) {
                window.os.entry(os).or_default().add(revenue);
            }
            if counts_for(RowFilter::Device) {
                window.devices.entry(device).or_default().add(revenue);
            }
            // Width breakdowns have no filter of their own, so any failure excludes the row.
            if failed != 0 {
                continue;
            }
            if screen_width > 0
                && let Some(bucket) = width_bucket(screen_width)
            {
                window
                    .screen_sizes
                    .entry(bucket.to_string())
                    .or_default()
                    .add(revenue);
            }
            if viewport_width > 0
                && let Some(bucket) = width_bucket(viewport_width)
            {
                window
                    .viewports
                    .entry(bucket.to_string())
                    .or_default()
                    .add(revenue);
            }
        }
        Ok(())
    })
    .map_err(map_row_error)?;

    if let Some(days) = &imported.current {
        add_imported_counts(conn, days, &mut current)?;
    }
    if let Some(days) = &imported.previous {
        add_imported_counts(conn, days, &mut previous)?;
    }

    Ok(AudienceResponse {
        countries: CountriesData {
            current: country_rows(current.countries, &previous.countries, factor),
        },
        cities: city_rows(
            current.cities,
            &previous.cities,
            factor,
            query.offset,
            query.order_by,
            query.direction,
        ),
        browsers: BrowsersData {
            current: browser_rows(current.browsers, &previous.browsers, factor),
        },
        os: OsData {
            current: os_rows(current.os, &previous.os, factor),
        },
        devices: DevicesData {
            current: device_rows(current.devices, &previous.devices, factor),
        },
        screen_sizes: ScreenSizesData {
            current: screen_size_rows(current.screen_sizes, &previous.screen_sizes, factor),
        },
        viewports: ViewportsData {
            current: viewport_rows(current.viewports, &previous.viewports, factor),
        },
    })
}

/// GA reports no screen or viewport widths, so those breakdowns stay tracked only.
fn add_imported_counts(
    conn: &ProjectConnection,
    days: &ImportedDays,
    window: &mut WindowCounts,
) -> Result<(), AuthError> {
    for place in days.places(conn)? {
        let country = Some(place.country);
        window
            .countries
            .entry(country.clone())
            .or_default()
            .add_imported(place.sessions);
        if let Some(city) = place.city {
            window
                .cities
                .entry(CityKey { country, city })
                .or_default()
                .add_imported(place.sessions);
        }
    }
    for row in days.technology(conn)? {
        window
            .browsers
            .entry(row.browser_family)
            .or_default()
            .add_imported(row.sessions);
        window
            .os
            .entry(row.os)
            .or_default()
            .add_imported(row.sessions);
        window
            .devices
            .entry(row.device)
            .or_default()
            .add_imported(row.sessions);
    }
    Ok(())
}

fn country_matches_segment(country: Option<&str>, segments: &SegmentFilters) -> bool {
    segments.countries.is_empty()
        || country.is_some_and(|country| {
            segments
                .countries
                .iter()
                .any(|selected| selected == country)
        })
}

/// Maps a landing width (screen or viewport) to a mutually exclusive analytics bucket.
fn width_bucket(width: i64) -> Option<&'static str> {
    match width {
        w if w < 360 => Some("small_mobile"),
        w if w < 768 => Some("mobile"),
        w if w < 1024 => Some("tablet"),
        w if w < 1440 => Some("desktop"),
        w if w < 1920 => Some("large_desktop"),
        _ => Some("x_large_desktop"),
    }
}
