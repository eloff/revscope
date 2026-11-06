//! Project timezones and the timezone-aware date bucketing built on them.
//!
//! Dashboard date ranges arrive as absolute instants (`[start, end)` in Unix
//! seconds), so *filtering* already honours whatever timezone the client picked
//! the range in and needs nothing from this module. What the project timezone is
//! needed for is *bucketing*: a chart's "day" has to run from local midnight to
//! local midnight and its "week" from local Monday to local Monday, or the
//! buckets will not line up with the dates the dashboard labels them with.
//!
//! Boundaries are resolved through a bundled IANA database rather than a stored
//! UTC offset, because an offset is only correct until the next DST transition:
//! a fixed shift would leave one 23- or 25-hour day in every affected range and
//! push every later boundary an hour off.
//!
//! Data that is *stored* pre-bucketed by UTC date (Search Console's daily rows)
//! cannot be re-bucketed at read time; see [`utc_days_covering`].

use std::sync::Arc;
use std::time::{Duration, Instant};

use jiff::civil::{Date, Time, Weekday};
use jiff::tz::TimeZone;
use jiff::{Span, Timestamp, ToSpan, Zoned};
use log::warn;

use crate::db::{DbError, ProjectCache, open_main_database_read};
use crate::download_db::DEMO_PROJECT_ID;
use crate::server_context::ServerContext;
use crate::util::{NANOS_PER_SECOND, block_on};

pub(crate) const DEFAULT_TIMEZONE: &str = "UTC";
/// Matches the `project_settings.timezone` column width. The longest real IANA
/// name is well under this; the cap only keeps junk out of the database.
pub(crate) const MAX_TIMEZONE_LEN: usize = 64;

/// How long a resolved zone is reused before it is read from Postgres again.
/// Saving a new zone clears the entry outright, so this is only a backstop for
/// changes made outside the settings route.
const CACHE_TTL: Duration = Duration::from_secs(5 * 60);

/// A project's timezone, already resolved against the IANA database.
///
/// Holding the parsed [`TimeZone`] (rather than the name) is what makes
/// [`validate`] a real check: a zone that reaches this type is one every
/// bucketing path can use.
#[derive(Clone, Debug)]
pub(crate) struct ProjectTimezone {
    name: String,
    zone: TimeZone,
}

impl ProjectTimezone {
    pub(crate) fn utc() -> Self {
        Self {
            name: DEFAULT_TIMEZONE.to_string(),
            zone: TimeZone::UTC,
        }
    }

    /// The IANA name as stored, suitable for writing back to the database.
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    /// Bucket boundaries covering `[start, end)`, aligned to this zone.
    ///
    /// The first boundary is the start of the bucket *containing* `start`, so it
    /// may precede the requested range — matching how the dashboard draws a
    /// partial leading bucket.
    pub(crate) fn buckets(&self, start: i64, end: i64, granularity: BucketGranularity) -> Buckets {
        let mut starts = Vec::new();
        if let Some(mut cursor) = self.aligned_start(start, granularity) {
            let step = granularity.step();
            while cursor.timestamp().as_second() < end {
                let second = cursor.timestamp().as_second();
                push_ascending(&mut starts, second);
                let Ok(next) = cursor.checked_add(step) else {
                    break;
                };
                // A zone whose offset jumps forward across the boundary can map
                // the next civil bucket onto the same instant; stopping keeps
                // `starts` strictly ascending for the binary search.
                if next.timestamp().as_second() <= second {
                    break;
                }
                cursor = next;
            }
        }
        Buckets { starts, end }
    }

    /// The `YYYY-MM-DD` calendar date `second` falls on in this zone.
    pub(crate) fn calendar_date(&self, second: i64) -> Option<String> {
        Some(self.zoned(second)?.date().to_string())
    }

    /// The calendar date `second` falls on in this zone.
    pub(crate) fn date_of(&self, second: i64) -> Option<Date> {
        Some(self.zoned(second)?.date())
    }

    /// First instant of `date` in this zone, in Unix seconds.
    pub(crate) fn day_start(&self, date: Date) -> Option<i64> {
        Some(
            date.to_zoned(self.zone.clone())
                .ok()?
                .timestamp()
                .as_second(),
        )
    }

    fn zoned(&self, second: i64) -> Option<Zoned> {
        Some(
            Timestamp::from_second(second)
                .ok()?
                .to_zoned(self.zone.clone()),
        )
    }

    /// Start of the bucket containing `second`, as a zoned instant.
    fn aligned_start(&self, second: i64, granularity: BucketGranularity) -> Option<Zoned> {
        let zoned = self.zoned(second)?;
        match granularity {
            BucketGranularity::Hourly => {
                let hour = Time::new(zoned.hour(), 0, 0, 0).ok()?;
                zoned
                    .date()
                    .to_datetime(hour)
                    .to_zoned(self.zone.clone())
                    .ok()
            }
            // `Date::to_zoned` resolves to the first instant of the day, so zones
            // that shift at midnight still get a boundary that exists.
            BucketGranularity::Daily => zoned.date().to_zoned(self.zone.clone()).ok(),
            BucketGranularity::Weekly => {
                let since_monday = i64::from(zoned.weekday().since(Weekday::Monday));
                let monday = zoned
                    .date()
                    .checked_sub(Span::new().days(since_monday))
                    .ok()?;
                monday.to_zoned(self.zone.clone()).ok()
            }
        }
    }
}

/// Chart bucket width. Weeks start on Monday in the project's timezone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum BucketGranularity {
    Hourly,
    #[default]
    Daily,
    Weekly,
}

impl BucketGranularity {
    /// Serialized granularity name the dashboard formats labels from.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Hourly => "hour",
            Self::Daily => "day",
            Self::Weekly => "week",
        }
    }

    /// Calendar step between boundaries. Applied to a zoned instant so a
    /// DST-shortened day still advances to the next local boundary.
    fn step(self) -> Span {
        match self {
            Self::Hourly => 1.hour(),
            Self::Daily => 1.day(),
            Self::Weekly => 7.days(),
        }
    }
}

pub(crate) fn parse_bucket_granularity(value: &str) -> Result<BucketGranularity, &'static str> {
    match value {
        "hourly" => Ok(BucketGranularity::Hourly),
        "daily" => Ok(BucketGranularity::Daily),
        "weekly" => Ok(BucketGranularity::Weekly),
        _ => Err("bucket must be hourly, daily, or weekly"),
    }
}

/// Ascending bucket boundaries for one range, and the instant they stop at.
///
/// Boundaries are irregularly spaced once DST is involved, so rows are assigned
/// by searching this list rather than by dividing the timestamp.
#[derive(Debug, Clone)]
pub(crate) struct Buckets {
    starts: Vec<i64>,
    end: i64,
}

impl Buckets {
    pub(crate) fn starts(&self) -> &[i64] {
        &self.starts
    }

    /// Start of the bucket containing `second`, or `None` when it falls outside
    /// the covered range.
    pub(crate) fn bucket_for_second(&self, second: i64) -> Option<i64> {
        self.index_for_second(second)
            .map(|index| self.starts[index])
    }

    /// [`Self::bucket_for_second`] for the nanosecond timestamps pageviews and
    /// sessions are keyed by.
    pub(crate) fn bucket_for_nanosecond(&self, nanosecond: i64) -> Option<i64> {
        self.bucket_for_second(nanosecond.div_euclid(NANOS_PER_SECOND))
    }

    /// Position of the bucket containing `second` within [`Self::starts`].
    pub(crate) fn index_for_second(&self, second: i64) -> Option<usize> {
        if second >= self.end {
            return None;
        }
        let after = self.starts.partition_point(|start| *start <= second);
        after.checked_sub(1)
    }
}

fn push_ascending(starts: &mut Vec<i64>, value: i64) {
    if starts.last().is_none_or(|last| *last < value) {
        starts.push(value);
    }
}

/// Widen `[start, end)` to the UTC calendar days it overlaps.
///
/// Google Search Console reports calendar dates, not instants, and we store one
/// row per UTC date. Those rows cannot be split, so a range picked in a
/// non-UTC zone is served by every UTC day it touches — a little wider than
/// asked for at each end rather than silently dropping a partial day. The same
/// widening is applied to the comparison window, so the day straddling the two
/// windows contributes to both.
pub(crate) fn utc_days_covering(start: i64, end: i64) -> (i64, i64) {
    const DAY_SECONDS: i64 = 86_400;
    let first = start.div_euclid(DAY_SECONDS) * DAY_SECONDS;
    let last = end.div_euclid(DAY_SECONDS) * DAY_SECONDS;
    let past_last = if last < end { last + DAY_SECONDS } else { last };
    (first, past_last.max(first + DAY_SECONDS))
}

/// Accept an IANA timezone name only if it resolves in the IANA database.
///
/// Validating by resolving is what keeps a typo from being stored: an unknown
/// name would otherwise pass a length check and then silently bucket every
/// chart in UTC for as long as it stayed saved.
pub(crate) fn validate(value: &str) -> Option<ProjectTimezone> {
    let name = value.trim();
    if name.is_empty() || name.len() > MAX_TIMEZONE_LEN {
        return None;
    }
    let zone = TimeZone::get(name).ok()?;
    Some(ProjectTimezone {
        name: name.to_string(),
        zone,
    })
}

/// TTL-cached project timezone, held on the project's [`ProjectCache`].
#[derive(Clone, Debug)]
pub(crate) struct ProjectTimezoneCache {
    timezone: ProjectTimezone,
    expires_at: Instant,
}

impl ProjectTimezoneCache {
    pub(crate) fn new(timezone: ProjectTimezone) -> Self {
        Self {
            timezone,
            expires_at: Instant::now() + CACHE_TTL,
        }
    }

    pub(crate) fn is_fresh(&self) -> bool {
        Instant::now() < self.expires_at
    }

    pub(crate) fn timezone(&self) -> &ProjectTimezone {
        &self.timezone
    }
}

/// The timezone a project's dashboards bucket and label dates in.
///
/// Cached per project rather than passed in with the request: the browser's copy
/// can be minutes stale after a teammate changes the setting, and the query and
/// Ask paths have no timezone to send. Falls back to UTC instead of failing the
/// read when the zone cannot be loaded, the same way ingest treats account
/// exclusions — a boundary an hour off beats a dead dashboard.
///
/// The public demo is always UTC: its page is unauthenticated, so it cannot read
/// the setting to compute matching ranges and labels, and its response cache is
/// keyed by range alone.
pub(crate) fn project_timezone(
    server_context: Arc<ServerContext>,
    project_key: i32,
) -> ProjectTimezone {
    let project = match crate::projects::fetch_project_blocking(server_context.clone(), project_key)
    {
        Ok(Some(project)) => project,
        Ok(None) => return ProjectTimezone::utc(),
        Err(error) => {
            warn!("failed to resolve project {project_key} for its timezone: {error}");
            return ProjectTimezone::utc();
        }
    };
    if Some(project.id()) == crate::projects::parse_project_id_str(DEMO_PROJECT_ID) {
        return ProjectTimezone::utc();
    }
    if let Some(cached) = project.timezone_if_fresh() {
        return cached;
    }
    match block_on(load_project_timezone(server_context, project.as_ref())) {
        Ok(timezone) => timezone,
        Err(error) => {
            warn!("failed to load timezone for project {project_key}: {error}");
            ProjectTimezone::utc()
        }
    }
}

/// Drops a project's cached zone so the next read picks up a freshly saved one.
pub(crate) fn invalidate(server_context: Arc<ServerContext>, project_key: i32) {
    if let Ok(Some(project)) = crate::projects::fetch_project_blocking(server_context, project_key)
    {
        project.clear_timezone();
    }
}

async fn load_project_timezone(
    server_context: Arc<ServerContext>,
    project: &ProjectCache,
) -> Result<ProjectTimezone, DbError> {
    let conn = open_main_database_read(server_context).await?;
    let row = conn
        .query_opt(
            "SELECT timezone FROM project_settings WHERE project_id = $1",
            &[&project.id()],
        )
        .await
        .map_err(DbError::main_postgres_query)?;
    drop(conn);

    let timezone = row
        .and_then(|row| validate(&row.get::<_, String>(0)))
        .unwrap_or_else(ProjectTimezone::utc);
    project.store_timezone(ProjectTimezoneCache::new(timezone.clone()));
    Ok(timezone)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;
    const HOUR: i64 = 3_600;

    fn zone(name: &str) -> ProjectTimezone {
        validate(name).expect("known zone")
    }

    /// Unix second for a UTC wall-clock instant, for readable fixtures.
    fn utc(text: &str) -> i64 {
        text.parse::<Timestamp>().expect("timestamp").as_second()
    }

    #[test]
    fn validate_requires_a_real_zone() {
        assert_eq!(validate("UTC").expect("utc").name(), "UTC");
        assert_eq!(
            validate(" America/New_York ").expect("trimmed").name(),
            "America/New_York"
        );
        assert!(validate("").is_none());
        assert!(validate("   ").is_none());
        assert!(validate("Mars/Olympus_Mons").is_none());
        // Passes the length check but is not a zone, which is exactly the case a
        // length-only check used to let through.
        assert!(validate("America/New_Yrok").is_none());
        assert!(validate(&"x".repeat(MAX_TIMEZONE_LEN + 1)).is_none());
    }

    #[test]
    fn daily_buckets_start_at_local_midnight() {
        let chicago = zone("America/Chicago");
        // 2026-06-10 00:00 UTC is 2026-06-09 19:00 in Chicago, so the first
        // boundary is the previous local midnight.
        let buckets = chicago.buckets(
            utc("2026-06-10T00:00:00Z"),
            utc("2026-06-12T00:00:00Z"),
            BucketGranularity::Daily,
        );
        assert_eq!(
            buckets.starts(),
            [
                utc("2026-06-09T05:00:00Z"),
                utc("2026-06-10T05:00:00Z"),
                utc("2026-06-11T05:00:00Z"),
            ]
        );
    }

    #[test]
    fn daily_buckets_absorb_dst_transitions() {
        let chicago = zone("America/Chicago");
        // Spring forward 2026-03-08: that local day is 23 hours long, so
        // dividing by 86_400 would drift every later boundary by an hour.
        let buckets = chicago.buckets(
            utc("2026-03-07T06:00:00Z"),
            utc("2026-03-10T06:00:00Z"),
            BucketGranularity::Daily,
        );
        assert_eq!(
            buckets.starts(),
            [
                utc("2026-03-07T06:00:00Z"),
                utc("2026-03-08T06:00:00Z"),
                utc("2026-03-09T05:00:00Z"),
                utc("2026-03-10T05:00:00Z"),
            ]
        );
        let short_day = buckets.starts()[2] - buckets.starts()[1];
        assert_eq!(short_day, 23 * HOUR);
        // Every later boundary stays on local midnight rather than inheriting
        // the shift.
        assert_eq!(buckets.starts()[3] - buckets.starts()[2], DAY);
    }

    /// The dashboard computes its own copy of these boundaries in
    /// `web/src/lib/timezone.ts` to build the range it asks for. The two have to
    /// agree exactly or a range will straddle the buckets drawn from it, so this
    /// pins the awkward zones both implementations have to get right.
    #[test]
    fn daily_boundaries_match_the_dashboards_own_calendar() {
        // Half-hour and quarter-hour standard offsets.
        assert_eq!(
            zone("Asia/Kolkata")
                .buckets(
                    utc("2026-06-10T12:00:00Z"),
                    utc("2026-06-10T13:00:00Z"),
                    BucketGranularity::Daily
                )
                .starts(),
            [utc("2026-06-09T18:30:00Z")]
        );
        assert_eq!(
            zone("Pacific/Chatham")
                .buckets(
                    utc("2026-06-10T12:00:00Z"),
                    utc("2026-06-10T13:00:00Z"),
                    BucketGranularity::Daily
                )
                .starts(),
            [utc("2026-06-10T11:15:00Z")]
        );
        // Lord Howe shifts by 30 minutes rather than a full hour.
        assert_eq!(
            zone("Australia/Lord_Howe")
                .buckets(
                    utc("2026-06-10T12:00:00Z"),
                    utc("2026-06-10T13:00:00Z"),
                    BucketGranularity::Daily
                )
                .starts(),
            [utc("2026-06-09T13:30:00Z")]
        );
        // Santiago springs forward at 00:00, so 2026-09-06 has no local midnight
        // and the day has to begin at 01:00 instead.
        let santiago = zone("America/Santiago");
        let gap_day = santiago.buckets(
            utc("2026-09-06T12:00:00Z"),
            utc("2026-09-06T13:00:00Z"),
            BucketGranularity::Daily,
        );
        assert_eq!(gap_day.starts(), [utc("2026-09-06T04:00:00Z")]);
        assert_eq!(
            santiago.calendar_date(gap_day.starts()[0]).as_deref(),
            Some("2026-09-06")
        );
    }

    #[test]
    fn weekly_buckets_start_on_local_monday() {
        let utc_zone = zone("UTC");
        // 2026-08-30 is a Sunday, so its week began Monday 2026-08-24.
        let buckets = utc_zone.buckets(
            utc("2026-08-30T12:00:00Z"),
            utc("2026-09-08T00:00:00Z"),
            BucketGranularity::Weekly,
        );
        assert_eq!(
            buckets.starts(),
            [
                utc("2026-08-24T00:00:00Z"),
                utc("2026-08-31T00:00:00Z"),
                utc("2026-09-07T00:00:00Z"),
            ]
        );
        for start in buckets.starts() {
            let weekday = Timestamp::from_second(*start)
                .expect("timestamp")
                .to_zoned(TimeZone::UTC)
                .weekday();
            assert_eq!(weekday, Weekday::Monday);
        }
    }

    #[test]
    fn hourly_buckets_align_to_local_hours() {
        // Kolkata is UTC+05:30, so local hour boundaries land on UTC half hours.
        let kolkata = zone("Asia/Kolkata");
        let buckets = kolkata.buckets(
            utc("2026-06-10T00:00:00Z"),
            utc("2026-06-10T02:00:00Z"),
            BucketGranularity::Hourly,
        );
        assert_eq!(
            buckets.starts(),
            [
                utc("2026-06-09T23:30:00Z"),
                utc("2026-06-10T00:30:00Z"),
                utc("2026-06-10T01:30:00Z"),
            ]
        );
    }

    #[test]
    fn assigns_rows_to_the_bucket_containing_them() {
        let chicago = zone("America/Chicago");
        let start = utc("2026-06-10T05:00:00Z");
        let end = utc("2026-06-13T05:00:00Z");
        let buckets = chicago.buckets(start, end, BucketGranularity::Daily);
        assert_eq!(buckets.starts().len(), 3);

        assert_eq!(buckets.bucket_for_second(start), Some(start));
        assert_eq!(
            buckets.bucket_for_second(utc("2026-06-11T04:59:59Z")),
            Some(start)
        );
        assert_eq!(
            buckets.bucket_for_second(utc("2026-06-11T05:00:00Z")),
            Some(utc("2026-06-11T05:00:00Z"))
        );
        // Nanosecond timestamps resolve the same way.
        assert_eq!(
            buckets.bucket_for_nanosecond(start * NANOS_PER_SECOND + 5),
            Some(start)
        );
        // Outside the covered range.
        assert_eq!(buckets.bucket_for_second(start - 1), None);
        assert_eq!(buckets.bucket_for_second(end), None);
    }

    #[test]
    fn empty_range_has_no_buckets() {
        let buckets = zone("UTC").buckets(DAY, DAY, BucketGranularity::Daily);
        assert!(buckets.starts().is_empty());
        assert_eq!(buckets.bucket_for_second(DAY), None);
    }

    #[test]
    fn calendar_date_is_local() {
        let chicago = zone("America/Chicago");
        // Still 2026-06-09 in Chicago when it is already the 10th in UTC.
        assert_eq!(
            chicago
                .calendar_date(utc("2026-06-10T04:00:00Z"))
                .as_deref(),
            Some("2026-06-09")
        );
        assert_eq!(
            chicago
                .calendar_date(utc("2026-06-10T06:00:00Z"))
                .as_deref(),
            Some("2026-06-10")
        );
    }

    #[test]
    fn utc_days_covering_widens_to_whole_days() {
        // A range already on UTC midnights is unchanged.
        assert_eq!(utc_days_covering(0, 2 * DAY), (0, 2 * DAY));
        // A range picked in UTC-5 starts and ends mid-UTC-day; both partial days
        // are included rather than dropped.
        assert_eq!(utc_days_covering(5 * HOUR, DAY + 5 * HOUR), (0, 2 * DAY));
        // Negative instants floor away from zero, not toward it.
        assert_eq!(utc_days_covering(-HOUR, HOUR), (-DAY, DAY));
        // A sub-day range still covers the day it sits in.
        assert_eq!(utc_days_covering(HOUR, 2 * HOUR), (0, DAY));
    }
}
