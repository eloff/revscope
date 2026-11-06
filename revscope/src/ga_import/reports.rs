//! What an import asks Google Analytics for, and how GA's values map onto ours.

use jiff::ToSpan;
use jiff::civil::Date;
use serde_json::{Value, json};

use crate::referrer::{
    ReferrerType, SourceGroup, classify_referrer_type, group_for_referrer_type,
    normalize_referrer_hostname,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReportKind {
    Totals,
    Pages,
    LandingPages,
    Sources,
    Geo,
    Technology,
    Events,
}

/// A running import's progress counts positions in this list, so changing its
/// order or length would skew imports in flight across a deploy.
pub(super) const REPORTS: [ReportKind; 7] = [
    ReportKind::Totals,
    ReportKind::Pages,
    ReportKind::LandingPages,
    ReportKind::Sources,
    ReportKind::Geo,
    ReportKind::Technology,
    ReportKind::Events,
];

/// The events GA collects by itself, for web and app streams. They describe activity
/// we already import or can't use, and aren't events the site chose to send.
const AUTOMATIC_EVENTS: [&str; 29] = [
    "page_view",
    "session_start",
    "first_visit",
    "user_engagement",
    "scroll",
    "click",
    "view_search_results",
    "file_download",
    "form_start",
    "form_submit",
    "video_start",
    "video_progress",
    "video_complete",
    "first_open",
    "screen_view",
    "app_update",
    "app_remove",
    "app_clear_data",
    "app_exception",
    "os_update",
    "notification_receive",
    "notification_open",
    "notification_dismiss",
    "notification_foreground",
    "ad_click",
    "ad_impression",
    "ad_exposure",
    "ad_query",
    "ad_reward",
];

impl ReportKind {
    /// Every report leads with `date`, which row parsing relies on.
    pub(super) fn dimensions(self) -> &'static [&'static str] {
        match self {
            Self::Totals => &["date"],
            Self::Pages => &["date", "hostName", "pagePath"],
            Self::LandingPages => &["date", "hostName", "landingPage"],
            Self::Sources => &[
                "date",
                "sessionSource",
                "sessionMedium",
                "sessionCampaignName",
                "sessionDefaultChannelGroup",
            ],
            Self::Geo => &["date", "countryId", "region", "city"],
            Self::Technology => &["date", "deviceCategory", "browser", "operatingSystem"],
            Self::Events => &["date", "eventName", "hostName", "pagePath"],
        }
    }

    pub(super) fn metrics(self) -> &'static [&'static str] {
        match self {
            Self::Totals => &[
                "sessions",
                "screenPageViews",
                "engagedSessions",
                "userEngagementDuration",
            ],
            Self::Pages => &["sessions", "screenPageViews", "userEngagementDuration"],
            Self::LandingPages => &["sessions", "engagedSessions", "userEngagementDuration"],
            Self::Sources | Self::Geo | Self::Technology => &["sessions"],
            Self::Events => &["sessions", "eventCount"],
        }
    }

    /// The `runReport` body for one month of this report. Paging is added by the caller.
    pub(super) fn request(self, from: Date, to: Date, hostnames: &[String]) -> Value {
        let mut filters = Vec::new();
        if !hostnames.is_empty() {
            filters.push(json!({
                "filter": {
                    "fieldName": "hostName",
                    "inListFilter": { "values": hostnames },
                }
            }));
        }
        if self == Self::Events {
            filters.push(json!({
                "notExpression": {
                    "filter": {
                        "fieldName": "eventName",
                        "inListFilter": { "values": AUTOMATIC_EVENTS },
                    }
                }
            }));
        }
        let mut request = json!({
            "dateRanges": [{ "startDate": from.to_string(), "endDate": to.to_string() }],
            "dimensions": self.dimensions().iter().map(|name| json!({ "name": name })).collect::<Vec<_>>(),
            "metrics": self.metrics().iter().map(|name| json!({ "name": name })).collect::<Vec<_>>(),
            "keepEmptyRows": false,
            // Without a total order, rows could move between the pages of a long
            // report, and a row seen twice would be added twice.
            "orderBys": self.dimensions().iter().map(|name| json!({ "dimension": { "dimensionName": name } })).collect::<Vec<_>>(),
        });
        match filters.len() {
            0 => {}
            1 => request["dimensionFilter"] = filters.remove(0),
            _ => {
                request["dimensionFilter"] = json!({ "andGroup": { "expressions": filters } });
            }
        }
        request
    }
}

/// One calendar month of the range, clipped to it, newest first so recent data
/// shows up soonest.
pub(super) fn months_newest_first(start: Date, end: Date) -> Vec<(Date, Date)> {
    let mut months = Vec::new();
    let mut month_end = end;
    while month_end >= start {
        let month_start = month_end.first_of_month().max(start);
        months.push((month_start, month_end));
        match month_start.checked_sub(1.day()) {
            Ok(previous) => month_end = previous,
            Err(_) => break,
        }
    }
    months
}

pub(super) fn chunk_count(start: Date, end: Date) -> i32 {
    (months_newest_first(start, end).len() * REPORTS.len()) as i32
}

/// The month and report at a position in an import's progress. `None` once past the end.
pub(super) fn chunk_at(start: Date, end: Date, index: i32) -> Option<((Date, Date), ReportKind)> {
    let index = usize::try_from(index).ok()?;
    let month = *months_newest_first(start, end).get(index / REPORTS.len())?;
    Some((month, REPORTS[index % REPORTS.len()]))
}

/// `YYYY-MM-DD` from GA's `YYYYMMDD`.
pub(super) fn parse_ga_date(value: &str) -> Option<String> {
    if value.len() != 8 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(format!("{}-{}-{}", &value[..4], &value[4..6], &value[6..]))
}

/// GA's stand-ins for a value it doesn't have, which we'd otherwise take for a
/// page or a place called that.
pub(super) fn known_value(value: &str) -> Option<&str> {
    let value = value.trim();
    match value {
        "" | "(not set)" | "(none)" | "(other)" | "(data not available)" | "not set" => None,
        _ => Some(value),
    }
}

/// A `paths` row's key, when GA's host and path can be one.
pub(super) fn page_key(hostname: &str, path: &str) -> Option<(String, String)> {
    let hostname = known_value(hostname)?.to_ascii_lowercase();
    let path = known_value(path).filter(|path| path.starts_with('/'))?;
    Some((hostname, path.to_string()))
}

/// GA's row for everything past its cardinality limit. Unlike a value it doesn't
/// know, this says nothing about the sessions' source, so it isn't direct traffic.
pub(super) fn is_rollup(value: &str) -> bool {
    value.trim() == "(other)"
}

#[derive(Debug, PartialEq)]
pub(super) enum ImportedSource {
    Direct,
    Referrer {
        host: String,
        channel: SourceGroup,
    },
    Campaign {
        source: String,
        medium: String,
        campaign: String,
    },
}

/// Sorts a GA traffic source into what ours would have recorded for the session.
pub(super) fn classify_source(
    source: &str,
    medium: &str,
    campaign: &str,
    channel_group: &str,
) -> ImportedSource {
    let medium = known_value(medium).unwrap_or("").to_ascii_lowercase();
    let source = known_value(source)
        .filter(|source| *source != "(direct)")
        .map(str::to_ascii_lowercase);
    // GA fills the campaign in with these for sessions that didn't have one.
    let campaign = known_value(campaign).filter(|campaign| {
        !matches!(
            *campaign,
            "(direct)" | "(organic)" | "(referral)" | "(cross-network)"
        )
    });
    if let Some(campaign) = campaign {
        return ImportedSource::Campaign {
            source: source.unwrap_or_default(),
            medium,
            campaign: campaign.to_string(),
        };
    }
    let Some(source) = source else {
        return ImportedSource::Direct;
    };
    let host = referrer_host(&source);
    let channel = match classify_referrer_type(Some(&host), None) {
        Some(referrer_type) => group_for_referrer_type(Some(referrer_type)),
        None => channel_from_ga_group(channel_group),
    };
    ImportedSource::Referrer { host, channel }
}

/// `google` for `google.com`: GA names the big search engines, and we store referrer hosts.
fn referrer_host(source: &str) -> String {
    match source {
        "google" | "bing" | "yahoo" | "baidu" | "yandex" | "duckduckgo" | "ecosia" => {
            format!("{source}.com")
        }
        _ => normalize_referrer_hostname(source).unwrap_or_else(|| source.to_string()),
    }
}

fn channel_from_ga_group(channel_group: &str) -> SourceGroup {
    match channel_group.trim().to_ascii_lowercase().as_str() {
        "organic search" => group_for_referrer_type(Some(ReferrerType::Search)),
        "organic social" => group_for_referrer_type(Some(ReferrerType::Social)),
        "paid search"
        | "paid social"
        | "paid video"
        | "paid shopping"
        | "paid other"
        | "display"
        | "cross-network"
        | "email"
        | "affiliates"
        | "sms"
        | "mobile push notifications" => SourceGroup::Campaigns,
        _ => SourceGroup::Referrers,
    }
}

/// `landings.device` for GA's `deviceCategory`.
pub(super) fn device_slug(value: &str) -> String {
    match known_value(value).map(str::to_ascii_lowercase).as_deref() {
        None => "unknown".to_string(),
        Some("smart tv") => "tv".to_string(),
        Some(other) => other.to_string(),
    }
}

/// `landings.browser_family` for GA's `browser`.
pub(super) fn browser_slug(value: &str) -> &'static str {
    let Some(value) = known_value(value).map(str::to_ascii_lowercase) else {
        return "unknown";
    };
    match value.as_str() {
        "chrome" | "android webview" => "chrome",
        "safari" | "safari (in-app)" => "safari",
        "firefox" => "firefox",
        "edge" => "edge",
        "internet explorer" => "ie",
        "opera" => "opera",
        "samsung internet" => "samsung",
        "uc browser" => "uc",
        "yandex" | "yabrowser" | "yandex browser" => "yandex",
        _ => "other",
    }
}

/// `landings.os` for GA's `operatingSystem`.
pub(super) fn os_slug(value: &str) -> &'static str {
    let Some(value) = known_value(value).map(str::to_ascii_lowercase) else {
        return "unknown";
    };
    match value.as_str() {
        "windows" => "windows",
        "macintosh" | "macos" | "mac os x" => "macos",
        "ios" => "ios",
        "android" => "android",
        "linux" => "linux",
        "chrome os" | "chromeos" => "chromeos",
        "kaios" => "kaios",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    fn months_run_newest_first_and_clip_to_the_range() {
        assert_eq!(
            months_newest_first(date(2026, 1, 15), date(2026, 3, 10)),
            vec![
                (date(2026, 3, 1), date(2026, 3, 10)),
                (date(2026, 2, 1), date(2026, 2, 28)),
                (date(2026, 1, 15), date(2026, 1, 31)),
            ]
        );
        assert_eq!(
            months_newest_first(date(2026, 3, 5), date(2026, 3, 5)),
            vec![(date(2026, 3, 5), date(2026, 3, 5))]
        );
    }

    #[test]
    fn chunks_visit_every_report_of_every_month_once() {
        let (start, end) = (date(2026, 1, 15), date(2026, 2, 10));
        let total = chunk_count(start, end);
        assert_eq!(total, 14);
        assert_eq!(
            chunk_at(start, end, 0),
            Some(((date(2026, 2, 1), date(2026, 2, 10)), ReportKind::Totals))
        );
        assert_eq!(
            chunk_at(start, end, 13),
            Some(((date(2026, 1, 15), date(2026, 1, 31)), ReportKind::Events))
        );
        assert_eq!(chunk_at(start, end, total), None);
        assert_eq!(chunk_at(start, end, -1), None);
    }

    #[test]
    fn every_request_filters_by_hostname_and_events_also_drop_automatic_ones() {
        let hosts = vec!["acme.test".to_string()];
        let from = date(2026, 1, 1);
        let plain = ReportKind::Totals.request(from, from, &[]);
        assert!(plain.get("dimensionFilter").is_none());
        let hosted = ReportKind::Geo.request(from, from, &hosts);
        assert_eq!(hosted["dimensionFilter"]["filter"]["fieldName"], "hostName");
        let events = ReportKind::Events.request(from, from, &[]);
        assert_eq!(
            events["dimensionFilter"]["notExpression"]["filter"]["fieldName"],
            "eventName"
        );
        let both = ReportKind::Events.request(from, from, &hosts);
        assert_eq!(
            both["dimensionFilter"]["andGroup"]["expressions"]
                .as_array()
                .map(Vec::len),
            Some(2)
        );
        assert_eq!(both["dateRanges"][0]["startDate"], "2026-01-01");
        assert_eq!(
            both["orderBys"].as_array().map(Vec::len),
            Some(ReportKind::Events.dimensions().len()),
            "every dimension takes part in the order, so pages can't overlap"
        );
    }

    #[test]
    fn ga_dates_become_iso_dates() {
        assert_eq!(parse_ga_date("20260131").as_deref(), Some("2026-01-31"));
        assert_eq!(parse_ga_date("(other)"), None);
    }

    #[test]
    fn only_real_pages_get_a_path() {
        assert_eq!(
            page_key("ACME.test", "/pricing"),
            Some(("acme.test".to_string(), "/pricing".to_string()))
        );
        assert_eq!(page_key("acme.test", "(not set)"), None);
        assert_eq!(page_key("acme.test", "(other)"), None);
        assert_eq!(page_key("(not set)", "/pricing"), None);
        assert_eq!(page_key("acme.test", "pricing"), None);
    }

    #[test]
    fn traffic_sources_sort_like_tracked_sessions() {
        assert_eq!(
            classify_source("(direct)", "(none)", "(direct)", "Direct"),
            ImportedSource::Direct
        );
        assert_eq!(
            classify_source("(not set)", "(not set)", "(not set)", "Unassigned"),
            ImportedSource::Direct
        );
        assert_eq!(
            classify_source("google", "organic", "(organic)", "Organic Search"),
            ImportedSource::Referrer {
                host: "google.com".to_string(),
                channel: SourceGroup::Search,
            }
        );
        assert_eq!(
            classify_source("www.example.org", "referral", "(referral)", "Referral"),
            ImportedSource::Referrer {
                host: "example.org".to_string(),
                channel: SourceGroup::Referrers,
            }
        );
        assert_eq!(
            classify_source("chatgpt.com", "referral", "(referral)", "Referral"),
            ImportedSource::Referrer {
                host: "chatgpt.com".to_string(),
                channel: SourceGroup::Ai,
            }
        );
        assert_eq!(
            classify_source("Newsletter", "Email", "launch", "Email"),
            ImportedSource::Campaign {
                source: "newsletter".to_string(),
                medium: "email".to_string(),
                campaign: "launch".to_string(),
            }
        );
    }

    #[test]
    fn ga_technology_names_map_to_landing_columns() {
        assert_eq!(device_slug("Smart TV"), "tv");
        assert_eq!(device_slug("mobile"), "mobile");
        assert_eq!(device_slug("(not set)"), "unknown");
        assert_eq!(browser_slug("Safari (in-app)"), "safari");
        assert_eq!(browser_slug("Samsung Internet"), "samsung");
        assert_eq!(browser_slug("Silk"), "other");
        assert_eq!(browser_slug("(not set)"), "unknown");
        assert_eq!(os_slug("Macintosh"), "macos");
        assert_eq!(os_slug("Chrome OS"), "chromeos");
        assert_eq!(os_slug("BeOS"), "unknown");
    }
}
