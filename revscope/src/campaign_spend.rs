//! Spend entered by hand on a campaign, and the totals ROAS and ROI use.
//!
//! An entry is either one amount on a date or a daily rate between two dates.
//! Totals only count days that have finished in the project's timezone: today
//! is still accruing spend, and its conversions have not had time to land.

use std::sync::Arc;

use fxhash::FxHashMap;
use jiff::civil::Date;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::auth::{AuthError, map_row_error};
use crate::authorize::WritableProject;
use crate::campaigns::in_transaction;
use crate::db::{ProjectConnection, open_project_database_write};
use crate::server_context::ServerContext;
use crate::timezone::ProjectTimezone;

const MAX_ENTRIES: usize = 50;
/// One billion in the project currency, in thousandths.
const MAX_AMOUNT: i64 = 1_000_000_000_000;
const MAX_RANGE_DAYS: i64 = 3_660;
const MIN_YEAR: i16 = 2000;
const MAX_YEAR: i16 = 2100;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SpendKind {
    Once,
    Daily,
}

impl SpendKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Once => "once",
            Self::Daily => "daily",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "once" => Some(Self::Once),
            "daily" => Some(Self::Daily),
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct SpendEntryInput {
    pub(crate) kind: SpendKind,
    #[serde(default)]
    pub(crate) start_date: String,
    #[serde(default)]
    pub(crate) end_date: Option<String>,
    pub(crate) amount: i64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SaveCampaignSpendRequest {
    pub(crate) campaign_id: i64,
    pub(crate) entries: Vec<SpendEntryInput>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SpendEntry {
    kind: SpendKind,
    start: Date,
    /// Inclusive. Equal to `start` for a single amount.
    end: Date,
    /// For `Daily`, the amount charged on each day.
    amount: i64,
}

#[derive(Debug)]
pub(crate) struct NormalizedSpendRequest {
    pub(crate) campaign_id: i64,
    pub(crate) entries: Vec<SpendEntry>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct CampaignSpendView {
    pub(crate) id: i64,
    pub(crate) kind: SpendKind,
    pub(crate) start_date: String,
    pub(crate) end_date: String,
    pub(crate) amount: i64,
    /// Spent on finished days, regardless of the dashboard range.
    pub(crate) spent_to_date: i64,
}

#[derive(Debug, Serialize)]
pub(crate) struct SaveCampaignSpendResponse {
    pub(crate) entries: usize,
}

/// One campaign's entries and what they add up to within a dashboard range.
#[derive(Debug, Default)]
pub(crate) struct CampaignSpend {
    pub(crate) entries: Vec<CampaignSpendView>,
    pub(crate) spend: i64,
}

/// Days `first <= day < cutoff` that spend is counted for.
#[derive(Clone, Copy, Debug)]
struct CountedDays {
    first: Date,
    cutoff: Date,
}

pub(crate) fn normalize_save_request(
    request: SaveCampaignSpendRequest,
) -> Result<NormalizedSpendRequest, &'static str> {
    if request.campaign_id <= 0 {
        return Err("campaign_id is required");
    }
    if request.entries.len() > MAX_ENTRIES {
        return Err("too many spend entries");
    }
    let mut entries = request
        .entries
        .into_iter()
        .map(normalize_entry)
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| (entry.start, entry.end));
    check_daily_overlap(&entries)?;
    Ok(NormalizedSpendRequest {
        campaign_id: request.campaign_id,
        entries,
    })
}

fn normalize_entry(input: SpendEntryInput) -> Result<SpendEntry, &'static str> {
    if input.amount <= 0 || input.amount > MAX_AMOUNT {
        return Err("spend amount is invalid");
    }
    let start = parse_date(&input.start_date).ok_or("spend date is invalid")?;
    let end = match input.kind {
        SpendKind::Once => start,
        SpendKind::Daily => {
            let end = input
                .end_date
                .as_deref()
                .and_then(parse_date)
                .ok_or("spend end date is invalid")?;
            if end < start {
                return Err("spend end date is before its start date");
            }
            if days_between(start, end) >= MAX_RANGE_DAYS {
                return Err("spend date range is too long");
            }
            end
        }
    };
    Ok(SpendEntry {
        kind: input.kind,
        start,
        end,
        amount: input.amount,
    })
}

fn parse_date(value: &str) -> Option<Date> {
    let value = value.trim();
    // Only the plain `YYYY-MM-DD` form, so stored dates sort as text.
    if value.len() != 10 {
        return None;
    }
    let date: Date = value.parse().ok()?;
    (MIN_YEAR..=MAX_YEAR).contains(&date.year()).then_some(date)
}

/// Daily ranges may not share a day; single amounts can land on any day.
/// Expects `entries` sorted by start date.
fn check_daily_overlap(entries: &[SpendEntry]) -> Result<(), &'static str> {
    let mut previous_end: Option<Date> = None;
    for entry in entries
        .iter()
        .filter(|entry| entry.kind == SpendKind::Daily)
    {
        if previous_end.is_some_and(|end| entry.start <= end) {
            return Err("spend date ranges overlap");
        }
        previous_end = Some(entry.end);
    }
    Ok(())
}

pub(crate) fn save_campaign_spend(
    server_context: Arc<ServerContext>,
    project: &WritableProject,
    request: NormalizedSpendRequest,
) -> Result<SaveCampaignSpendResponse, AuthError> {
    let conn = open_project_database_write(server_context.as_ref(), project.id())
        .map_err(|error| AuthError::Db(error.to_string()))?;
    save_campaign_spend_on(&conn, &request)
}

fn save_campaign_spend_on(
    conn: &ProjectConnection,
    request: &NormalizedSpendRequest,
) -> Result<SaveCampaignSpendResponse, AuthError> {
    in_transaction(conn, |conn| {
        let exists = conn
            .query_row(
                "SELECT 1 FROM campaigns WHERE id = ?1 AND deleted_at IS NULL",
                params![request.campaign_id],
                |_| Ok(()),
            )
            .optional()
            .map_err(map_row_error)?;
        if exists.is_none() {
            return Err(AuthError::InvalidRequest("campaign not found"));
        }
        conn.execute(
            "DELETE FROM campaign_spend WHERE campaign_id = ?1",
            params![request.campaign_id],
        )
        .map_err(map_row_error)?;
        for entry in &request.entries {
            conn.execute(
                "INSERT INTO campaign_spend (campaign_id, kind, start_date, end_date, amount)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    request.campaign_id,
                    entry.kind.as_str(),
                    entry.start.to_string(),
                    entry.end.to_string(),
                    entry.amount
                ],
            )
            .map_err(map_row_error)?;
        }
        Ok(SaveCampaignSpendResponse {
            entries: request.entries.len(),
        })
    })
}

/// Entries for every visible campaign, with spend summed over the finished
/// days that start inside `[start, end)`.
pub(crate) fn load_campaign_spend(
    conn: &ProjectConnection,
    timezone: &ProjectTimezone,
    start: i64,
    end: i64,
    now: i64,
) -> Result<FxHashMap<i64, CampaignSpend>, AuthError> {
    let rows = conn
        .with_query(
            "SELECT s.id, s.campaign_id, s.kind, s.start_date, s.end_date, s.amount
             FROM campaign_spend s
             JOIN campaigns c ON c.id = s.campaign_id AND c.deleted_at IS NULL
             ORDER BY s.campaign_id, s.start_date, s.id",
            [],
            |mut rows| {
                let mut loaded = Vec::new();
                while let Some(row) = rows.next()? {
                    let kind: String = row.get(2)?;
                    let start_date: String = row.get(3)?;
                    let end_date: String = row.get(4)?;
                    let entry = SpendKind::parse(&kind).and_then(|kind| {
                        Some(SpendEntry {
                            kind,
                            start: start_date.parse().ok()?,
                            end: end_date.parse().ok()?,
                            amount: row.get(5).ok()?,
                        })
                    });
                    if let Some(entry) = entry {
                        loaded.push((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, entry));
                    }
                }
                Ok(loaded)
            },
        )
        .map_err(map_row_error)?;
    if rows.is_empty() {
        return Ok(FxHashMap::default());
    }

    let Some(today) = timezone.date_of(now) else {
        return Ok(FxHashMap::default());
    };
    let to_date = CountedDays {
        first: Date::MIN,
        cutoff: today,
    };
    let in_range = range_days(timezone, start, end, today);
    let mut spend: FxHashMap<i64, CampaignSpend> = FxHashMap::default();
    for (id, campaign_id, entry) in rows {
        let campaign = spend.entry(campaign_id).or_default();
        if let Some(days) = in_range {
            campaign.spend = campaign.spend.saturating_add(entry_spend(&entry, days));
        }
        campaign.entries.push(CampaignSpendView {
            id,
            kind: entry.kind,
            start_date: entry.start.to_string(),
            end_date: entry.end.to_string(),
            amount: entry.amount,
            spent_to_date: entry_spend(&entry, to_date),
        });
    }
    Ok(spend)
}

/// Days whose local start falls in `[start, end)`, stopping before `today`.
fn range_days(
    timezone: &ProjectTimezone,
    start: i64,
    end: i64,
    today: Date,
) -> Option<CountedDays> {
    let first = first_day_starting_at_or_after(timezone, start)?;
    let today_start = timezone.day_start(today)?;
    let cutoff = if end < today_start {
        first_day_starting_at_or_after(timezone, end)?
    } else {
        today
    };
    Some(CountedDays { first, cutoff })
}

fn first_day_starting_at_or_after(timezone: &ProjectTimezone, second: i64) -> Option<Date> {
    let date = timezone.date_of(second)?;
    if timezone.day_start(date)? < second {
        date.tomorrow().ok()
    } else {
        Some(date)
    }
}

fn entry_spend(entry: &SpendEntry, days: CountedDays) -> i64 {
    let first = entry.start.max(days.first);
    let Ok(after_end) = entry.end.tomorrow() else {
        return 0;
    };
    let cutoff = after_end.min(days.cutoff);
    if first >= cutoff {
        return 0;
    }
    match entry.kind {
        SpendKind::Once => entry.amount,
        SpendKind::Daily => entry.amount.saturating_mul(days_between(first, cutoff)),
    }
}

fn days_between(start: Date, end: Date) -> i64 {
    start
        .until(end)
        .map(|span| i64::from(span.get_days()))
        .unwrap_or(0)
}

/// Revenue per unit of spend, or `None` without spend to divide by.
pub(crate) fn roas(revenue: i64, spend: i64) -> Option<f64> {
    (spend > 0).then(|| revenue as f64 / spend as f64)
}

/// Profit over spend, or `None` without spend to divide by.
pub(crate) fn roi(revenue: i64, spend: i64) -> Option<f64> {
    (spend > 0).then(|| (revenue - spend) as f64 / spend as f64)
}

#[cfg(test)]
mod tests {
    use crate::test_utils::open_migrated_db;

    use super::*;

    /// The project comes from the path, and the dashboard sends no `project_id`.
    #[test]
    fn spend_payload_deserializes_without_a_project_id() {
        serde_json::from_str::<SaveCampaignSpendRequest>(
            r#"{"campaign_id":5,"entries":[{"kind":"once","start_date":"2026-01-01","amount":500}]}"#,
        )
        .expect("spend payload");
    }

    fn insert_campaign(conn: &ProjectConnection) -> i64 {
        conn.query_row(
            "INSERT INTO campaigns (source, medium, campaign, first_seen_at)
             VALUES ('newsletter', 'email', 'launch', 1)
             RETURNING id",
            [],
            |row| row.get(0),
        )
        .expect("campaign")
    }

    fn once(date: &str, amount: i64) -> SpendEntryInput {
        SpendEntryInput {
            kind: SpendKind::Once,
            start_date: date.to_string(),
            end_date: None,
            amount,
        }
    }

    fn daily(start: &str, end: &str, amount: i64) -> SpendEntryInput {
        SpendEntryInput {
            kind: SpendKind::Daily,
            start_date: start.to_string(),
            end_date: Some(end.to_string()),
            amount,
        }
    }

    fn request(campaign_id: i64, entries: Vec<SpendEntryInput>) -> SaveCampaignSpendRequest {
        SaveCampaignSpendRequest {
            campaign_id,
            entries,
        }
    }

    fn utc_second(date: &str) -> i64 {
        let date: Date = date.parse().expect("date");
        ProjectTimezone::utc().day_start(date).expect("day start")
    }

    #[test]
    fn rejects_overlapping_daily_ranges() {
        let error = normalize_save_request(request(
            1,
            vec![
                daily("2026-03-10", "2026-03-20", 1_000),
                daily("2026-03-01", "2026-03-10", 1_000),
            ],
        ))
        .expect_err("overlap");
        assert_eq!(error, "spend date ranges overlap");
    }

    #[test]
    fn allows_adjacent_ranges_and_single_amounts_inside_them() {
        let normalized = normalize_save_request(request(
            1,
            vec![
                daily("2026-03-11", "2026-03-20", 1_000),
                daily("2026-03-01", "2026-03-10", 1_000),
                once("2026-03-05", 50_000),
            ],
        ))
        .expect("valid");
        assert_eq!(normalized.entries.len(), 3);
    }

    #[test]
    fn rejects_bad_dates_and_amounts() {
        let backwards =
            normalize_save_request(request(1, vec![daily("2026-03-10", "2026-03-01", 1_000)]));
        assert_eq!(
            backwards.expect_err("backwards"),
            "spend end date is before its start date"
        );
        let missing_end = normalize_save_request(request(
            1,
            vec![SpendEntryInput {
                kind: SpendKind::Daily,
                start_date: "2026-03-01".to_string(),
                end_date: None,
                amount: 1_000,
            }],
        ));
        assert_eq!(
            missing_end.expect_err("no end"),
            "spend end date is invalid"
        );
        let zero = normalize_save_request(request(1, vec![once("2026-03-01", 0)]));
        assert_eq!(zero.expect_err("zero"), "spend amount is invalid");
        let junk = normalize_save_request(request(1, vec![once("March 1", 1_000)]));
        assert_eq!(junk.expect_err("junk"), "spend date is invalid");
    }

    #[test]
    fn counts_finished_days_inside_the_range() {
        let conn = open_migrated_db();
        let campaign_id = insert_campaign(&conn);
        let normalized = normalize_save_request(request(
            campaign_id,
            vec![
                daily("2026-03-01", "2026-03-31", 40_000),
                once("2026-03-05", 300_000),
                once("2026-03-20", 900_000),
            ],
        ))
        .expect("valid");
        save_campaign_spend_on(&conn, &normalized).expect("save");

        // Range covers March 1–14; it is now noon on March 10, so March 10
        // onward has not finished yet.
        let now = utc_second("2026-03-10") + 12 * 3_600;
        let spend = load_campaign_spend(
            &conn,
            &ProjectTimezone::utc(),
            utc_second("2026-03-01"),
            utc_second("2026-03-15"),
            now,
        )
        .expect("load");
        let campaign = spend.get(&campaign_id).expect("campaign spend");
        assert_eq!(campaign.spend, 9 * 40_000 + 300_000);
        let daily_entry = campaign
            .entries
            .iter()
            .find(|entry| entry.kind == SpendKind::Daily)
            .expect("daily");
        assert_eq!(daily_entry.spent_to_date, 9 * 40_000);
        let future = campaign
            .entries
            .iter()
            .find(|entry| entry.start_date == "2026-03-20")
            .expect("future single");
        assert_eq!(future.spent_to_date, 0);
    }

    #[test]
    fn range_starting_mid_day_skips_that_day() {
        let conn = open_migrated_db();
        let campaign_id = insert_campaign(&conn);
        let normalized = normalize_save_request(request(
            campaign_id,
            vec![daily("2026-03-01", "2026-03-31", 1_000)],
        ))
        .expect("valid");
        save_campaign_spend_on(&conn, &normalized).expect("save");
        let spend = load_campaign_spend(
            &conn,
            &ProjectTimezone::utc(),
            utc_second("2026-03-02") + 3_600,
            utc_second("2026-03-05"),
            utc_second("2026-04-10"),
        )
        .expect("load");
        assert_eq!(spend[&campaign_id].spend, 2 * 1_000);
    }

    #[test]
    fn saving_replaces_entries_and_requires_a_campaign() {
        let conn = open_migrated_db();
        let campaign_id = insert_campaign(&conn);
        let first = normalize_save_request(request(
            campaign_id,
            vec![once("2026-03-01", 1_000), once("2026-03-02", 1_000)],
        ))
        .expect("valid");
        save_campaign_spend_on(&conn, &first).expect("save");
        let second = normalize_save_request(request(campaign_id, vec![once("2026-03-03", 5_000)]))
            .expect("valid");
        save_campaign_spend_on(&conn, &second).expect("save");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM campaign_spend", [], |row| row.get(0))
            .expect("count");
        assert_eq!(count, 1);

        let missing = normalize_save_request(request(campaign_id + 1, vec![])).expect("valid");
        assert!(save_campaign_spend_on(&conn, &missing).is_err());
    }

    #[test]
    fn ratios_need_spend() {
        assert_eq!(roas(10_000, 0), None);
        assert_eq!(roi(10_000, 0), None);
        assert_eq!(roas(10_000, 2_000), Some(5.0));
        assert_eq!(roi(10_000, 2_000), Some(4.0));
    }
}
