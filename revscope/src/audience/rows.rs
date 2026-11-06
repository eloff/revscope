//! Turning per-dimension counts into the audience report's ranked rows.

use std::cmp::Ordering;
use std::hash::Hash;

use fxhash::FxHashMap;
use serde::Serialize;

use crate::comparison::Change;
use crate::country_region::country_name_for_country;
use crate::landing_labels::{browser_family_name, device_name, os_name};
use crate::metrics::{
    self, WilsonInterval, conversion_rate_wilson_interval, conversion_rate_wilson_lower_bound,
};
use crate::query_params::{SortDirection, paginate};

/// City rows returned per page.
pub(crate) const CITIES_PAGE_SIZE: i64 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CitiesOrderBy {
    Name,
    Visitors,
    Revenue,
}

impl CitiesOrderBy {
    pub(super) fn parse(value: &str) -> Result<Self, &'static str> {
        match value {
            "name" => Ok(Self::Name),
            "visitors" => Ok(Self::Visitors),
            "revenue" => Ok(Self::Revenue),
            _ => Err("invalid order_by"),
        }
    }
}

/// Display-ready metrics shared by audience breakdown rows.
///
/// Counts and revenue (`visitors`, `converting_visitors`, `revenue`) are raw
/// current-period values. `revenue_per_visitor` is derived from the current
/// period. The `*_change` fields are period-over-period ratios versus the
/// comparison window (the prior 30 days), with additive quantities normalized
/// to the current period length.
///
/// Conversion rates are country-only (see [`CountryRow`]); other dimensions
/// omit them because sparse buckets make the rate mostly noise.
#[derive(Debug, Serialize)]
pub(crate) struct RowMetrics {
    /// First visits (landings) in the period for this dimension value, imported ones included.
    pub(crate) visitors: i64,
    /// The visitors tracked by us, the only ones with revenue to convert into.
    #[serde(skip)]
    pub(crate) tracked_visitors: i64,
    /// First visits with attributed payment revenue.
    pub(crate) converting_visitors: i64,
    /// Sum of qualifying `payments.amount` (thousandths) attributed to this value.
    /// Excludes subscription renewals.
    pub(crate) revenue: i64,
    /// Revenue ÷ tracked visitors (thousandths).
    pub(crate) revenue_per_visitor: f64,
    /// Normalized revenue change versus the comparison window.
    pub(crate) revenue_change: Change,
    /// Normalized visitor change versus the comparison window.
    pub(crate) visitors_change: Change,
    /// Normalized conversion-count change versus the comparison window.
    pub(crate) conversions_change: Change,
}

/// Per-country breakdown row.
#[derive(Debug, Serialize)]
pub(crate) struct CountryRow {
    /// ISO 3166-1 alpha-2 code (uppercase) from `landings.country`. Absent for
    /// landings with no resolved country.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) country: Option<String>,
    /// Human-readable country display name resolved from `country`.
    pub(crate) country_name: String,
    #[serde(flatten)]
    pub(crate) metrics: RowMetrics,
    /// Converting visitors ÷ tracked visitors.
    pub(crate) conversion_rate: f64,
    /// Lower bound of the 95% Wilson interval, used for ranking.
    pub(crate) conversion_lower_bound: f64,
    /// 95% Wilson confidence interval for the conversion rate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) conversion_ci: Option<WilsonInterval>,
}

/// Country breakdown for the requested window. Per-row deltas compare against
/// the prior 30 days (normalized), computed server-side.
#[derive(Debug, Serialize)]
pub(crate) struct CountriesData {
    pub(crate) current: Vec<CountryRow>,
}

/// Per-city breakdown row.
#[derive(Debug, Serialize)]
pub(crate) struct CityRow {
    /// City name (English) from `landings.city`.
    pub(crate) city: String,
    /// ISO 3166-1 alpha-2 code from `landings.country` for this city row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) country: Option<String>,
    #[serde(flatten)]
    pub(crate) metrics: RowMetrics,
}

/// City breakdown for the requested window, paginated.
///
/// Ranked by the requested `order_by` (default: revenue, visitors, then city
/// name). Page size is [`CITIES_PAGE_SIZE`].
#[derive(Debug, Serialize)]
pub(crate) struct CitiesData {
    pub(crate) current: Vec<CityRow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) next_offset: Option<i64>,
    pub(crate) offset: i64,
    pub(crate) limit: i64,
}

/// Per-browser-family breakdown row.
#[derive(Debug, Serialize)]
pub(crate) struct BrowserRow {
    /// Stored browser family slug (`chrome`, `firefox`, …) or `unknown` when
    /// `landings.browser_family` is null.
    pub(crate) browser_family: String,
    /// Human-readable browser family display name.
    pub(crate) browser_family_name: String,
    #[serde(flatten)]
    pub(crate) metrics: RowMetrics,
}

/// Browser-family breakdown for the requested window.
#[derive(Debug, Serialize)]
pub(crate) struct BrowsersData {
    pub(crate) current: Vec<BrowserRow>,
}

/// Per-OS breakdown row.
#[derive(Debug, Serialize)]
pub(crate) struct OsRow {
    /// Stored OS slug (`windows`, `macos`, …) or `unknown` when `landings.os`
    /// is null.
    pub(crate) os: String,
    /// Human-readable OS display name.
    pub(crate) os_name: String,
    #[serde(flatten)]
    pub(crate) metrics: RowMetrics,
}

/// OS breakdown for the requested window.
#[derive(Debug, Serialize)]
pub(crate) struct OsData {
    pub(crate) current: Vec<OsRow>,
}

/// Per-device breakdown row.
#[derive(Debug, Serialize)]
pub(crate) struct DeviceRow {
    /// Stored device slug (`desktop`, `mobile`, …).
    pub(crate) device: String,
    /// Human-readable device display name.
    pub(crate) device_name: String,
    #[serde(flatten)]
    pub(crate) metrics: RowMetrics,
}

/// Device breakdown for the requested window.
#[derive(Debug, Serialize)]
pub(crate) struct DevicesData {
    pub(crate) current: Vec<DeviceRow>,
}

/// Per screen-width bucket breakdown row.
#[derive(Debug, Serialize)]
pub(crate) struct ScreenSizeRow {
    /// Bucket slug (`small_mobile`, `mobile`, …).
    pub(crate) screen_size: String,
    #[serde(flatten)]
    pub(crate) metrics: RowMetrics,
}

/// Screen-width bucket breakdown for the requested window.
#[derive(Debug, Serialize)]
pub(crate) struct ScreenSizesData {
    pub(crate) current: Vec<ScreenSizeRow>,
}

/// Per viewport-width bucket breakdown row.
#[derive(Debug, Serialize)]
pub(crate) struct ViewportRow {
    /// Bucket slug (`small_mobile`, `mobile`, …).
    pub(crate) viewport: String,
    #[serde(flatten)]
    pub(crate) metrics: RowMetrics,
}

/// Viewport-width bucket breakdown for the requested window.
#[derive(Debug, Serialize)]
pub(crate) struct ViewportsData {
    pub(crate) current: Vec<ViewportRow>,
}

/// Accumulated visitor and revenue counts for a single dimension value within
/// one window.
#[derive(Default)]
pub(super) struct Counts {
    visitors: i64,
    converting_visitors: i64,
    revenue: i64,
    imported_visitors: i64,
}

impl Counts {
    pub(super) fn add_imported(&mut self, sessions: i64) {
        self.imported_visitors += sessions;
    }

    fn total_visitors(&self) -> i64 {
        self.visitors + self.imported_visitors
    }

    pub(super) fn add(&mut self, revenue: i64) {
        self.visitors += 1;
        if revenue > 0 {
            self.converting_visitors += 1;
        }
        self.revenue += revenue;
    }
}

/// Grouping key for city breakdown rows (country + city name).
#[derive(Eq, Hash, PartialEq)]
pub(super) struct CityKey {
    pub(super) country: Option<String>,
    pub(super) city: String,
}

/// Counts per value of one dimension, in one window.
pub(super) type DimensionCounts<K> = FxHashMap<K, Counts>;

/// Computes the shared per-row metrics for a current-period dimension value,
/// comparing against the matched previous-period counts (when present).
fn row_metrics(current: &Counts, previous: Option<&Counts>, factor: f64) -> RowMetrics {
    let visitors = current.total_visitors();
    let converting = current.converting_visitors;
    let change = |current: i64, previous: Option<i64>| {
        Change::breakdown_absolute(current as f64, previous.map(|p| p as f64), factor)
    };
    RowMetrics {
        visitors,
        tracked_visitors: current.visitors,
        converting_visitors: converting,
        revenue: current.revenue,
        revenue_per_visitor: metrics::revenue_per_visitor(
            current.revenue as f64,
            current.visitors as f64,
        ),
        revenue_change: change(current.revenue, previous.map(|p| p.revenue)),
        visitors_change: change(visitors, previous.map(Counts::total_visitors)),
        conversions_change: change(converting, previous.map(|p| p.converting_visitors)),
    }
}

/// Each value's metrics against the previous window, ranked by revenue, then
/// visitors, then the value ascending (an unknown `None` value first).
fn ranked<K: Eq + Hash + Ord>(
    counts: DimensionCounts<K>,
    previous: &DimensionCounts<K>,
    factor: f64,
) -> Vec<(K, RowMetrics)> {
    let mut rows: Vec<(K, RowMetrics)> = counts
        .into_iter()
        .map(|(key, counts)| {
            let metrics = row_metrics(&counts, previous.get(&key), factor);
            (key, metrics)
        })
        .collect();
    rows.sort_by(|(a_key, a), (b_key, b)| {
        b.revenue
            .cmp(&a.revenue)
            .then_with(|| b.visitors.cmp(&a.visitors))
            .then_with(|| a_key.cmp(b_key))
    });
    rows
}

pub(super) fn country_rows(
    counts: DimensionCounts<Option<String>>,
    previous: &DimensionCounts<Option<String>>,
    factor: f64,
) -> Vec<CountryRow> {
    ranked(counts, previous, factor)
        .into_iter()
        .map(|(country, metrics)| {
            let converting = metrics.converting_visitors as f64;
            let visitors = metrics.tracked_visitors as f64;
            CountryRow {
                country_name: country_name_for_country(country.as_deref()),
                country,
                conversion_rate: metrics::conversion_rate(converting, visitors),
                conversion_lower_bound: conversion_rate_wilson_lower_bound(converting, visitors),
                conversion_ci: conversion_rate_wilson_interval(converting, visitors),
                metrics,
            }
        })
        .collect()
}

pub(super) fn browser_rows(
    counts: DimensionCounts<String>,
    previous: &DimensionCounts<String>,
    factor: f64,
) -> Vec<BrowserRow> {
    ranked(counts, previous, factor)
        .into_iter()
        .map(|(browser_family, metrics)| BrowserRow {
            browser_family_name: browser_family_name(&browser_family),
            browser_family,
            metrics,
        })
        .collect()
}

pub(super) fn os_rows(
    counts: DimensionCounts<String>,
    previous: &DimensionCounts<String>,
    factor: f64,
) -> Vec<OsRow> {
    ranked(counts, previous, factor)
        .into_iter()
        .map(|(os, metrics)| OsRow {
            os_name: os_name(&os),
            os,
            metrics,
        })
        .collect()
}

pub(super) fn device_rows(
    counts: DimensionCounts<String>,
    previous: &DimensionCounts<String>,
    factor: f64,
) -> Vec<DeviceRow> {
    ranked(counts, previous, factor)
        .into_iter()
        .map(|(device, metrics)| DeviceRow {
            device_name: device_name(&device),
            device,
            metrics,
        })
        .collect()
}

pub(super) fn screen_size_rows(
    counts: DimensionCounts<String>,
    previous: &DimensionCounts<String>,
    factor: f64,
) -> Vec<ScreenSizeRow> {
    ranked(counts, previous, factor)
        .into_iter()
        .map(|(screen_size, metrics)| ScreenSizeRow {
            screen_size,
            metrics,
        })
        .collect()
}

pub(super) fn viewport_rows(
    counts: DimensionCounts<String>,
    previous: &DimensionCounts<String>,
    factor: f64,
) -> Vec<ViewportRow> {
    ranked(counts, previous, factor)
        .into_iter()
        .map(|(viewport, metrics)| ViewportRow { viewport, metrics })
        .collect()
}

/// Converts grouped city counts into ranked, paginated rows.
///
/// Ranked by the requested column (visitors break ties except when sorting by
/// visitors, which breaks ties on revenue; then city name ascending). Only
/// landings with a non-null `city` are included; each row carries the landing's
/// ISO country code when present.
pub(super) fn city_rows(
    counts: DimensionCounts<CityKey>,
    previous: &DimensionCounts<CityKey>,
    factor: f64,
    offset: i64,
    order_by: CitiesOrderBy,
    direction: SortDirection,
) -> CitiesData {
    let mut out: Vec<CityRow> = counts
        .into_iter()
        .map(|(key, c)| {
            let metrics = row_metrics(&c, previous.get(&key), factor);
            CityRow {
                city: key.city,
                country: key.country,
                metrics,
            }
        })
        .collect();
    out.sort_by(|a, b| cmp_city_rows(a, b, order_by, direction));
    let (page, next_offset) = paginate(out, offset, CITIES_PAGE_SIZE);
    CitiesData {
        current: page,
        next_offset,
        offset,
        limit: CITIES_PAGE_SIZE,
    }
}

fn cmp_city_rows(
    a: &CityRow,
    b: &CityRow,
    order_by: CitiesOrderBy,
    direction: SortDirection,
) -> Ordering {
    let primary = match order_by {
        CitiesOrderBy::Name => cmp_city_name(a, b),
        CitiesOrderBy::Visitors => a.metrics.visitors.cmp(&b.metrics.visitors),
        CitiesOrderBy::Revenue => a.metrics.revenue.cmp(&b.metrics.revenue),
    };
    let primary = direction.apply(primary);
    if primary != Ordering::Equal {
        return primary;
    }
    if matches!(order_by, CitiesOrderBy::Visitors) {
        let revenue = b.metrics.revenue.cmp(&a.metrics.revenue);
        if revenue != Ordering::Equal {
            return revenue;
        }
    } else {
        let visitors = b.metrics.visitors.cmp(&a.metrics.visitors);
        if visitors != Ordering::Equal {
            return visitors;
        }
    }
    cmp_city_name(a, b)
}

fn cmp_city_name(a: &CityRow, b: &CityRow) -> Ordering {
    a.city
        .to_ascii_lowercase()
        .cmp(&b.city.to_ascii_lowercase())
        .then_with(|| a.country.cmp(&b.country))
}
