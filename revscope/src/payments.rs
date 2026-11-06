use std::sync::Arc;

use rusqlite::types::ToSql;
use rusqlite::{params, params_from_iter};
use serde::Serialize;
use serde_json::Value;

use crate::auth::{AuthError, map_row_error};
use crate::authorize::AuthorizedProject;
use crate::db::{ProjectConnection, open_project_database_read};
use crate::query_params::{parse_positive_id, parse_report_query};
use crate::referrer::ReferrerType;
use crate::segment_filter::{SegmentFilters, parse_segment_filters};
use crate::server_context::ServerContext;
use crate::util::NANOS_PER_SECOND;

const DEFAULT_LIMIT: i64 = 100;
const MAX_LIMIT: i64 = 1_000;

#[derive(Debug, Serialize)]
pub(crate) struct ConversionsResponse {
    pub(crate) conversions: Vec<PaymentEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct PaymentEntry {
    pub(crate) id: i64,
    pub(crate) provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) subscription_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) provider_subscription_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) interval_unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) visitor_timestamp_id: Option<i64>,
    pub(crate) provider_payment_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) provider_order_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) provider_invoice_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) provider_customer_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) customer_email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) order_email: Option<String>,
    pub(crate) payment_type: String,
    pub(crate) status: String,
    pub(crate) amount: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) mrr_amount: Option<i64>,
    pub(crate) currency: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) fee_amount: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) net_amount: Option<i64>,
    pub(crate) paid_at: i64,
    pub(crate) created_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) metadata: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) landing: Option<PaymentLanding>,
}

#[derive(Debug, Serialize)]
pub(crate) struct PaymentLanding {
    pub(crate) visitor_timestamp_id: i64,
    pub(crate) viewed_at: i64,
    pub(crate) session_duration_seconds: i64,
    pub(crate) session_page_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) click_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) referrer_type: Option<ReferrerType>,
    pub(crate) is_google: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) timezone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) country: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) city: Option<String>,
    pub(crate) tracker_version: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) os: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) os_version: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) browser_family: Option<String>,
    pub(crate) device: String,
    pub(crate) screen_width: i64,
    pub(crate) screen_height: i64,
    pub(crate) viewport_width: i64,
    pub(crate) viewport_height: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) referrer: Option<PaymentReferrer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) campaign: Option<PaymentCampaign>,
}

#[derive(Debug, Serialize)]
pub(crate) struct PaymentReferrer {
    pub(crate) id: i64,
    pub(crate) referrer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) deleted_at: Option<i64>,
}

#[derive(Debug, Serialize)]
pub(crate) struct PaymentCampaign {
    pub(crate) id: i64,
    pub(crate) source: String,
    pub(crate) medium: String,
    pub(crate) campaign: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) network: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) display_name: Option<String>,
    pub(crate) first_seen_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) deleted_at: Option<i64>,
}

/// Fallback label when source/medium/campaign/network are all empty.
pub(crate) const UNNAMED_CAMPAIGN_LABEL: &str = "(unnamed campaign)";

/// Single-line campaign label: prefers `display_name`, otherwise joins non-empty
/// attribution fields as `key=value` pairs (mirrors web `campaignDisplayLabel`).
pub(crate) fn campaign_display_label(campaign: &PaymentCampaign) -> String {
    if let Some(display_name) = campaign.display_name.as_deref()
        && !display_name.is_empty()
    {
        return display_name.to_string();
    }
    let mut parts = Vec::new();
    if let Some(network) = campaign.network.as_deref()
        && !network.is_empty()
        && network != "generic"
    {
        parts.push(format!("ads={network}"));
    }
    if !campaign.source.is_empty() {
        parts.push(format!("source={}", campaign.source));
    }
    if !campaign.medium.is_empty() {
        parts.push(format!("medium={}", campaign.medium));
    }
    if !campaign.campaign.is_empty() {
        parts.push(format!("campaign={}", campaign.campaign));
    }
    if parts.is_empty() {
        return UNNAMED_CAMPAIGN_LABEL.to_string();
    }
    parts.join(", ")
}

/// Landing-attribution bucket for revenue-source drill-down. Mirrors
/// `revenue_source.rs` bucketing: campaign wins over referrer, then direct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RevenueSourceFilter {
    Campaign(i64),
    Referrer(i64),
    Direct,
}

pub(crate) struct PaymentsQuery {
    pub(crate) start: i64,
    pub(crate) end: i64,
    pub(crate) limit: i64,
    pub(crate) cursor: Option<PaymentsCursor>,
    pub(crate) path_id: Option<i64>,
    pub(crate) landing_path_id: Option<i64>,
    pub(crate) revenue_source: Option<RevenueSourceFilter>,
    pub(crate) segments: SegmentFilters,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PaymentsCursor {
    paid_at: i64,
    id: i64,
}

pub(crate) fn parse_payments_query(target: &str) -> Result<PaymentsQuery, &'static str> {
    let mut limit = DEFAULT_LIMIT;
    let mut cursor = None;
    let mut path_id = None;
    let mut landing_path_id = None;
    let mut revenue_source = None;
    let range = parse_report_query(target, |key, value| {
        let source = match key {
            "limit" => {
                let parsed = value.parse::<i64>().map_err(|_| "limit must be a number")?;
                if parsed < 1 {
                    return Err("limit must be at least 1");
                }
                limit = parsed.min(MAX_LIMIT);
                None
            }
            "cursor" => {
                cursor = Some(parse_cursor(value)?);
                None
            }
            "path_id" => {
                path_id = Some(parse_positive_id(value, "invalid path_id")?);
                None
            }
            "landing_path_id" => {
                landing_path_id = Some(parse_positive_id(value, "invalid landing_path_id")?);
                None
            }
            "referrer_id" => Some(RevenueSourceFilter::Referrer(parse_positive_id(
                value,
                "invalid referrer_id",
            )?)),
            "campaign_id" => Some(RevenueSourceFilter::Campaign(parse_positive_id(
                value,
                "invalid campaign_id",
            )?)),
            "direct" => {
                if value != "1" && !value.eq_ignore_ascii_case("true") {
                    return Err("direct must be 1 or true");
                }
                Some(RevenueSourceFilter::Direct)
            }
            _ => None,
        };
        if source.is_some() {
            if revenue_source.is_some() {
                return Err("only one revenue source filter may be set");
            }
            revenue_source = source;
        }
        Ok(())
    })?;
    let (start, end) = range.validate()?;
    Ok(PaymentsQuery {
        start,
        end,
        limit,
        cursor,
        path_id,
        landing_path_id,
        revenue_source,
        segments: parse_segment_filters(target),
    })
}

fn revenue_source_filter_params(
    filter: Option<RevenueSourceFilter>,
) -> (Option<i64>, Option<i64>, i64) {
    match filter {
        None => (None, None, 0),
        Some(RevenueSourceFilter::Campaign(id)) => (Some(id), None, 1),
        Some(RevenueSourceFilter::Referrer(id)) => (None, Some(id), 1),
        Some(RevenueSourceFilter::Direct) => (None, None, 1),
    }
}

pub(crate) fn conversions(
    server_context: Arc<ServerContext>,
    project: &AuthorizedProject,
    query: PaymentsQuery,
) -> Result<ConversionsResponse, AuthError> {
    fetch_conversions(server_context, project.id(), &query)
}

fn fetch_conversions(
    server_context: Arc<ServerContext>,
    project_id: &str,
    query: &PaymentsQuery,
) -> Result<ConversionsResponse, AuthError> {
    let conn = open_project_database_read(server_context.as_ref(), project_id)
        .map_err(|error| AuthError::Db(error.to_string()))?;
    build_conversions_response(&conn, query)
}

pub(crate) fn build_conversions_response(
    conn: &ProjectConnection,
    query: &PaymentsQuery,
) -> Result<ConversionsResponse, AuthError> {
    let mut entries = fetch_payment_entries(conn, query)?;
    let has_more = entries.len() as i64 > query.limit;
    if has_more {
        entries.truncate(query.limit as usize);
    }
    let next_cursor = if has_more {
        entries
            .last()
            .map(|entry| format!("{}:{}", entry.paid_at, entry.id))
    } else {
        None
    };
    Ok(ConversionsResponse {
        conversions: entries,
        next_cursor,
    })
}

fn fetch_payment_entries(
    conn: &ProjectConnection,
    query: &PaymentsQuery,
) -> Result<Vec<PaymentEntry>, AuthError> {
    // Fixed binds occupy ?1..?13; country placeholders begin at ?14. Country
    // filters on the joined landing row (`l`), so only payments attributed to a
    // visitor who landed from the selected countries are returned.
    let country_filter = query.segments.country_in_clause("l.country", 14);
    let session_filter = query
        .segments
        .session_filter_clause("p.visitor_timestamp_id", None);
    let sql = format!(
        "SELECT
            p.id,
            p.provider,
            p.subscription_id,
            p.interval_unit,
            p.visitor_timestamp_id,
            p.provider_payment_id,
            p.provider_order_id,
            p.provider_invoice_id,
            p.provider_customer_id,
            p.customer_email,
            p.order_email,
            p.payment_type,
            p.status,
            p.amount,
            p.mrr_amount,
            p.currency,
            p.fee_amount,
            p.net_amount,
            p.paid_at,
            p.created_at,
            p.metadata,
            l.visitor_timestamp_id,
            l.visitor_timestamp_id / ?6 AS viewed_at,
            l.session_duration_seconds,
            l.session_page_count,
            l.click_id,
            l.referrer_type,
            l.is_google,
            l.timezone,
            l.language,
            l.country,
            l.city,
            l.tracker_version,
            l.os,
            l.os_version,
            l.browser_family,
            l.device,
            l.screen_width,
            l.screen_height,
            l.viewport_width,
            l.viewport_height,
            r.id,
            r.referrer,
            r.deleted_at,
            c.id,
            c.source,
            c.medium,
            c.campaign,
            c.network,
            c.display_name,
            c.first_seen_at,
            c.deleted_at,
            s.provider_subscription_id
         FROM payments p
         LEFT JOIN subscriptions s ON s.id = p.subscription_id
         LEFT JOIN landings l ON l.visitor_timestamp_id = p.visitor_timestamp_id
         LEFT JOIN referrers r ON r.id = l.referrer
         LEFT JOIN campaigns c ON c.id = l.campaign
         WHERE p.status = 'ok'
           AND p.payment_type IN ('single', 'subscription')
           AND NOT (
               p.payment_type = 'subscription'
               AND EXISTS (
                   SELECT 1
                   FROM payments prior
                   WHERE prior.subscription_id = p.subscription_id
                     AND prior.status = 'ok'
                     AND prior.payment_type = 'subscription'
                     AND prior.paid_at < p.paid_at
                     AND prior.id != p.id
               )
           )
           -- Page/landing drilldowns scope the date range to the pageview or
           -- landing cohort (matching Pages / Landing Pages). Otherwise the
           -- range is cash-basis on paid_at.
           AND (
               ?7 IS NOT NULL
               OR ?8 IS NOT NULL
               OR (p.paid_at >= ?1 AND p.paid_at < ?2)
           )
           AND (?3 IS NULL OR p.paid_at < ?3 OR (p.paid_at = ?3 AND p.id > ?4))
           AND (
               ?7 IS NULL
               OR EXISTS (
                   SELECT 1
                   FROM pageviews path_pv
                   WHERE path_pv.visitor_timestamp_id = p.visitor_timestamp_id
                     AND path_pv.path_id = ?7
                     AND path_pv.timestamp_id >= ?9
                     AND path_pv.timestamp_id < ?10
                     AND path_pv.timestamp_id / ?6 <= p.paid_at
               )
           )
           AND (
               ?8 IS NULL
               OR EXISTS (
                   SELECT 1
                   FROM landings landing
                   JOIN pageviews landing_pv
                     ON landing_pv.timestamp_id = landing.visitor_timestamp_id
                   WHERE landing.visitor_timestamp_id = p.visitor_timestamp_id
                     AND landing_pv.path_id = ?8
                     AND landing.visitor_timestamp_id >= ?9
                     AND landing.visitor_timestamp_id < ?10
                     AND p.paid_at >= landing.visitor_timestamp_id / ?6
               )
           )
           AND (
               ?13 = 0
               OR (
                   l.visitor_timestamp_id IS NOT NULL
                   AND l.visitor_timestamp_id >= ?9
                   AND l.visitor_timestamp_id < ?10
                   AND (
                       (?11 IS NOT NULL AND l.campaign = ?11 AND c.deleted_at IS NULL)
                       OR (?12 IS NOT NULL AND (c.id IS NULL OR c.deleted_at IS NOT NULL) AND l.referrer = ?12)
                       OR (?13 = 1 AND ?11 IS NULL AND ?12 IS NULL AND (c.id IS NULL OR c.deleted_at IS NOT NULL) AND l.referrer IS NULL)
                   )
               )
           ){country_filter}{session_filter}
         ORDER BY p.paid_at DESC, p.id ASC
         LIMIT ?5"
    );

    let (cursor_paid_at, cursor_id) = query
        .cursor
        .map(|cursor| (Some(cursor.paid_at), Some(cursor.id)))
        .unwrap_or((None, None));
    let window_start_nanos = query.start * NANOS_PER_SECOND;
    let window_end_nanos = query.end * NANOS_PER_SECOND;
    let (campaign_id, referrer_id, revenue_source_active) =
        revenue_source_filter_params(query.revenue_source);
    let limit_plus_one = query.limit + 1;
    let nanos = NANOS_PER_SECOND;
    let mut sql_params: Vec<&dyn ToSql> = vec![
        &query.start,
        &query.end,
        &cursor_paid_at,
        &cursor_id,
        &limit_plus_one,
        &nanos,
        &query.path_id,
        &query.landing_path_id,
        &window_start_nanos,
        &window_end_nanos,
        &campaign_id,
        &referrer_id,
        &revenue_source_active,
    ];
    query.segments.append_country_params(&mut sql_params);
    conn.with_query(&sql, params_from_iter(sql_params), |mut rows| {
        let mut out = Vec::with_capacity(query.limit.min(MAX_LIMIT) as usize);
        while let Some(row) = rows.next()? {
            out.push(payment_entry_from_row(row)?);
        }
        Ok(out)
    })
    .map_err(map_row_error)
}

pub(crate) fn fetch_payment_entry_by_id(
    conn: &ProjectConnection,
    payment_id: i64,
) -> Result<Option<PaymentEntry>, AuthError> {
    let sql = "SELECT
            p.id,
            p.provider,
            p.subscription_id,
            p.interval_unit,
            p.visitor_timestamp_id,
            p.provider_payment_id,
            p.provider_order_id,
            p.provider_invoice_id,
            p.provider_customer_id,
            p.customer_email,
            p.order_email,
            p.payment_type,
            p.status,
            p.amount,
            p.mrr_amount,
            p.currency,
            p.fee_amount,
            p.net_amount,
            p.paid_at,
            p.created_at,
            p.metadata,
            l.visitor_timestamp_id,
            l.visitor_timestamp_id / ?2 AS viewed_at,
            l.session_duration_seconds,
            l.session_page_count,
            l.click_id,
            l.referrer_type,
            l.is_google,
            l.timezone,
            l.language,
            l.country,
            l.city,
            l.tracker_version,
            l.os,
            l.os_version,
            l.browser_family,
            l.device,
            l.screen_width,
            l.screen_height,
            l.viewport_width,
            l.viewport_height,
            r.id,
            r.referrer,
            r.deleted_at,
            c.id,
            c.source,
            c.medium,
            c.campaign,
            c.network,
            c.display_name,
            c.first_seen_at,
            c.deleted_at,
            s.provider_subscription_id
         FROM payments p
         LEFT JOIN subscriptions s ON s.id = p.subscription_id
         LEFT JOIN landings l ON l.visitor_timestamp_id = p.visitor_timestamp_id
         LEFT JOIN referrers r ON r.id = l.referrer
         LEFT JOIN campaigns c ON c.id = l.campaign
         WHERE p.id = ?1
           AND p.status = 'ok'
           AND p.payment_type IN ('single', 'subscription')
           AND NOT (
               p.payment_type = 'subscription'
               AND EXISTS (
                   SELECT 1
                   FROM payments prior
                   WHERE prior.subscription_id = p.subscription_id
                     AND prior.status = 'ok'
                     AND prior.payment_type = 'subscription'
                     AND prior.paid_at < p.paid_at
                     AND prior.id != p.id
               )
           )
         LIMIT 1";
    conn.with_query(sql, params![payment_id, NANOS_PER_SECOND], |mut rows| {
        rows.next()?.map(payment_entry_from_row).transpose()
    })
    .map_err(map_row_error)
}

fn payment_entry_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PaymentEntry> {
    Ok(PaymentEntry {
        id: row.get(0)?,
        provider: row.get(1)?,
        subscription_id: row.get(2)?,
        provider_subscription_id: row.get(52)?,
        interval_unit: row.get(3)?,
        visitor_timestamp_id: row.get(4)?,
        provider_payment_id: row.get(5)?,
        provider_order_id: row.get(6)?,
        provider_invoice_id: row.get(7)?,
        provider_customer_id: row.get(8)?,
        customer_email: row.get(9)?,
        order_email: row.get(10)?,
        payment_type: row.get(11)?,
        status: row.get(12)?,
        amount: row.get(13)?,
        mrr_amount: row.get(14)?,
        currency: row.get(15)?,
        fee_amount: row.get(16)?,
        net_amount: row.get(17)?,
        paid_at: row.get(18)?,
        created_at: row.get(19)?,
        metadata: parse_optional_json(row.get(20)?),
        landing: build_landing(row)?,
    })
}

fn build_landing(row: &rusqlite::Row<'_>) -> rusqlite::Result<Option<PaymentLanding>> {
    let visitor_timestamp_id = row.get::<_, Option<i64>>(21)?;
    let Some(visitor_timestamp_id) = visitor_timestamp_id else {
        return Ok(None);
    };
    let referrer_type = row
        .get::<_, Option<String>>(26)?
        .and_then(|value| ReferrerType::from_stored(&value));
    Ok(Some(PaymentLanding {
        visitor_timestamp_id,
        viewed_at: row.get(22)?,
        session_duration_seconds: row.get(23)?,
        session_page_count: row.get(24)?,
        click_id: row.get(25)?,
        referrer_type,
        is_google: row.get::<_, i64>(27)? != 0,
        timezone: row.get(28)?,
        language: row.get(29)?,
        country: row.get(30)?,
        city: row.get(31)?,
        tracker_version: row.get(32)?,
        os: row.get(33)?,
        os_version: row.get(34)?,
        browser_family: row.get(35)?,
        device: row.get(36)?,
        screen_width: row.get(37)?,
        screen_height: row.get(38)?,
        viewport_width: row.get(39)?,
        viewport_height: row.get(40)?,
        referrer: build_referrer(row)?,
        campaign: build_campaign(row)?,
    }))
}

fn build_referrer(row: &rusqlite::Row<'_>) -> rusqlite::Result<Option<PaymentReferrer>> {
    let id = row.get::<_, Option<i64>>(41)?;
    Ok(id.map(|id| PaymentReferrer {
        id,
        referrer: row.get(42).unwrap_or_default(),
        deleted_at: row.get(43).unwrap_or(None),
    }))
}

fn build_campaign(row: &rusqlite::Row<'_>) -> rusqlite::Result<Option<PaymentCampaign>> {
    let id = row.get::<_, Option<i64>>(44)?;
    Ok(id.map(|id| PaymentCampaign {
        id,
        source: row.get(45).unwrap_or_default(),
        medium: row.get(46).unwrap_or_default(),
        campaign: row.get(47).unwrap_or_default(),
        network: row.get(48).unwrap_or(None),
        display_name: row.get(49).unwrap_or(None),
        first_seen_at: row.get(50).unwrap_or_default(),
        deleted_at: row.get(51).unwrap_or(None),
    }))
}

fn parse_cursor(value: &str) -> Result<PaymentsCursor, &'static str> {
    let decoded = value.replace("%3A", ":").replace("%3a", ":");
    let Some((paid_at, id)) = decoded.split_once(':') else {
        return Err("cursor must be paid_at:id");
    };
    let paid_at = paid_at
        .parse::<i64>()
        .map_err(|_| "cursor must be paid_at:id")?;
    let id = id.parse::<i64>().map_err(|_| "cursor must be paid_at:id")?;
    Ok(PaymentsCursor { paid_at, id })
}

fn parse_optional_json(value: Option<String>) -> Option<Value> {
    value.and_then(|value| serde_json::from_str(&value).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::open_migrated_db;

    use crate::db::ProjectConnection;

    use crate::test_utils::{
        PaymentFixture, SubscriptionFixture, insert_payment as insert_test_payment,
        insert_subscription as insert_test_subscription, visitor_timestamp_id_for_page_id,
    };
    use rusqlite::params;

    fn page_id(seed: u8) -> [u8; 16] {
        [seed; 16]
    }

    fn timestamp_id(seed: u8, viewed_at: i64) -> i64 {
        viewed_at * NANOS_PER_SECOND + i64::from(seed)
    }

    fn insert_path(conn: &ProjectConnection) -> i64 {
        insert_path_with_url(conn, "/", "example.com")
    }

    fn insert_path_with_url(conn: &ProjectConnection, path: &str, hostname: &str) -> i64 {
        conn.execute(
            "INSERT INTO paths (path, hostname) VALUES (?1, ?2)",
            params![path, hostname],
        )
        .expect("insert path");
        conn.query_row(
            "SELECT id FROM paths WHERE path = ?1 AND hostname = ?2",
            params![path, hostname],
            |row| row.get::<_, i64>(0),
        )
        .expect("path id")
    }

    fn insert_referrer(conn: &ProjectConnection, referrer: &str) -> i64 {
        conn.execute(
            "INSERT INTO referrers (referrer, deleted_at) VALUES (?1, NULL)",
            params![referrer],
        )
        .expect("insert referrer");
        conn.query_row(
            "SELECT id FROM referrers ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("referrer id")
    }

    fn insert_campaign(conn: &ProjectConnection) -> i64 {
        conn.execute(
            "INSERT INTO campaigns (
                source, medium, campaign, network, display_name, first_seen_at, deleted_at
             ) VALUES ('google', 'cpc', 'spring', 'google_ads', 'Spring', 90, NULL)",
            [],
        )
        .expect("insert campaign");
        conn.query_row(
            "SELECT id FROM campaigns ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("campaign id")
    }

    fn insert_landing(
        conn: &ProjectConnection,
        path_id: i64,
        seed: u8,
        viewed_at: i64,
        referrer: Option<i64>,
        campaign: Option<i64>,
    ) {
        let page_id = page_id(seed);
        let timestamp_id = timestamp_id(seed, viewed_at);
        conn.execute(
            "INSERT INTO pageviews (
                timestamp_id, visitor_timestamp_id, previous_timestamp_id, path_id,
                duration_seconds, scrolled_percent
             ) VALUES (?1, ?1, NULL, ?2, 5, NULL)",
            params![timestamp_id, path_id],
        )
        .expect("insert pageview");
        conn.execute(
            "INSERT INTO client_page_ids_map (visitor_timestamp_id, page_id)
             VALUES (?1, ?2)",
            params![timestamp_id, page_id.as_slice()],
        )
        .expect("insert page id map");
        conn.execute(
            "INSERT INTO landings (
                visitor_timestamp_id, session_duration_seconds, session_page_count,
                referrer, campaign, click_id, referrer_type, is_google, timezone,
                language, country, city, tracker_version, os, os_version, browser_family,
                device, screen_width, screen_height, viewport_width, viewport_height
             ) VALUES (
                ?1, 5, 1,
                ?2, ?3, 'gclid-1', 'search', 1, 'America/Chicago',
                'EN', 'US', 'Austin', 1.0, 'linux', 6.0, 'chrome',
                'desktop', 1920, 1080, 1440, 900
             )",
            params![timestamp_id, referrer, campaign],
        )
        .expect("insert landing");
    }

    fn insert_pageview(
        conn: &ProjectConnection,
        seed: u8,
        viewed_at: i64,
        session_seed: u8,
        session_viewed_at: i64,
        path_id: i64,
    ) {
        let pageview_id = timestamp_id(seed, viewed_at);
        let visitor_timestamp_id = timestamp_id(session_seed, session_viewed_at);
        conn.execute(
            "INSERT INTO pageviews (
                timestamp_id, visitor_timestamp_id, previous_timestamp_id, path_id,
                duration_seconds, scrolled_percent
             ) VALUES (?1, ?2, NULL, ?3, 5, NULL)",
            params![pageview_id, visitor_timestamp_id, path_id],
        )
        .expect("insert pageview");
    }

    fn insert_payment(
        conn: &ProjectConnection,
        provider_payment_id: &str,
        first_page_seed: Option<u8>,
        amount: i64,
        paid_at: i64,
    ) {
        let visitor_timestamp_id = first_page_seed.map(page_id).map(|id| {
            visitor_timestamp_id_for_page_id(conn, id.as_slice()).expect("mapped landing timestamp")
        });
        let mut payment = PaymentFixture::new(provider_payment_id, amount, paid_at);
        payment.visitor_timestamp_id = visitor_timestamp_id;
        payment.provider_order_id = Some("order_1".to_string());
        payment.provider_invoice_id = Some("invoice_1".to_string());
        payment.provider_customer_id = Some("customer_1".to_string());
        payment.customer_email = Some("buyer@example.com".to_string());
        payment.fee_amount = Some(100);
        payment.net_amount = Some(amount - 100);
        payment.metadata = Some(r#"{"sku":"pro"}"#.to_string());
        insert_test_payment(conn, payment);
    }

    fn baseline_query(start: i64, end: i64) -> PaymentsQuery {
        PaymentsQuery {
            start,
            end,
            limit: 100,
            cursor: None,
            path_id: None,
            landing_path_id: None,
            revenue_source: None,
            segments: SegmentFilters::default(),
        }
    }

    #[test]
    fn parses_limit_and_cursor() {
        let query = parse_payments_query(
            "/v2/read/projects/1/conversions?start=1&end=100&limit=2000&cursor=50:9",
        )
        .expect("query");
        assert_eq!(query.limit, MAX_LIMIT);
        assert_eq!(query.cursor, Some(PaymentsCursor { paid_at: 50, id: 9 }));
        assert_eq!(query.path_id, None);
        assert_eq!(query.landing_path_id, None);
    }

    #[test]
    fn parses_optional_path_filters() {
        let query = parse_payments_query(
            "/v2/read/projects/1/conversions?start=1&end=100&path_id=12&landing_path_id=34",
        )
        .expect("query");

        assert_eq!(query.path_id, Some(12));
        assert_eq!(query.landing_path_id, Some(34));
    }

    #[test]
    fn rejects_invalid_path_filters() {
        assert!(
            parse_payments_query("/v2/read/projects/1/conversions?start=1&end=100&path_id=0")
                .is_err()
        );
        assert!(
            parse_payments_query(
                "/v2/read/projects/1/conversions?start=1&end=100&landing_path_id=bad"
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_invalid_cursor() {
        assert!(
            parse_payments_query("/v2/read/projects/1/conversions?start=1&end=100&cursor=bad")
                .is_err()
        );
    }

    #[test]
    fn accepts_url_encoded_cursor_separator() {
        let query =
            parse_payments_query("/v2/read/projects/1/conversions?start=1&end=100&cursor=50%3A9")
                .expect("query");
        assert_eq!(query.cursor, Some(PaymentsCursor { paid_at: 50, id: 9 }));
    }

    #[test]
    fn returns_payments_with_landing_attribution() {
        let conn = open_migrated_db();
        let path_id = insert_path(&conn);
        let referrer = insert_referrer(&conn, "google.com/search");
        let campaign = insert_campaign(&conn);
        insert_landing(&conn, path_id, 1, 100, Some(referrer), Some(campaign));
        insert_payment(&conn, "pay_1", Some(1), 25_000, 200);

        let response =
            build_conversions_response(&conn, &baseline_query(0, 1_000)).expect("response");

        assert_eq!(response.conversions.len(), 1);
        assert_eq!(response.next_cursor, None);
        let payment = &response.conversions[0];
        assert_eq!(payment.provider_payment_id, "pay_1");
        assert_eq!(payment.amount, 25_000);
        assert_eq!(payment.visitor_timestamp_id, Some(timestamp_id(1, 100)));
        assert_eq!(
            payment.metadata.as_ref().and_then(|value| value.get("sku")),
            Some(&Value::String("pro".to_string()))
        );
        let landing = payment.landing.as_ref().expect("landing");
        assert_eq!(landing.visitor_timestamp_id, timestamp_id(1, 100));
        assert_eq!(landing.viewed_at, 100);
        assert_eq!(landing.referrer_type, Some(ReferrerType::Search));
        assert!(landing.is_google);
        assert_eq!(landing.country.as_deref(), Some("US"));
        assert_eq!(
            landing.referrer.as_ref().map(|row| row.referrer.as_str()),
            Some("google.com/search")
        );
        assert_eq!(
            landing.campaign.as_ref().map(|row| row.campaign.as_str()),
            Some("spring")
        );
    }

    fn insert_subscription(conn: &ProjectConnection) -> i64 {
        let mut subscription = SubscriptionFixture::new("sub_1");
        subscription.provider_customer_id = Some("cus_1".to_string());
        subscription.customer_email = Some("sub@example.com".to_string());
        subscription.current_period_start = Some(100);
        subscription.current_period_end = Some(200);
        subscription.started_at = Some(100);
        insert_test_subscription(conn, subscription)
    }

    fn insert_subscription_payment(
        conn: &ProjectConnection,
        subscription_id: i64,
        provider_payment_id: &str,
        amount: i64,
        paid_at: i64,
    ) {
        let mut payment = PaymentFixture::new(provider_payment_id, amount, paid_at);
        payment.subscription_id = Some(subscription_id);
        payment.interval_unit = Some("month".to_string());
        payment.provider_order_id = Some("order_1".to_string());
        payment.provider_invoice_id = Some("invoice_1".to_string());
        payment.provider_customer_id = Some("cus_1".to_string());
        payment.customer_email = Some("sub@example.com".to_string());
        payment.payment_type = "subscription".to_string();
        payment.mrr_amount = Some(amount);
        payment.fee_amount = Some(100);
        payment.net_amount = Some(amount - 100);
        insert_test_payment(conn, payment);
    }

    #[test]
    fn excludes_recurring_subscription_payments() {
        let conn = open_migrated_db();
        let subscription_id = insert_subscription(&conn);
        insert_subscription_payment(&conn, subscription_id, "pay_new", 10_000, 100);
        insert_subscription_payment(&conn, subscription_id, "pay_renew", 10_000, 200);

        let response =
            build_conversions_response(&conn, &baseline_query(0, 1_000)).expect("response");

        assert_eq!(response.conversions.len(), 1);
        assert_eq!(response.conversions[0].provider_payment_id, "pay_new");
    }

    #[test]
    fn paginates_by_paid_at_desc_and_id_asc() {
        let conn = open_migrated_db();
        insert_payment(&conn, "pay_1", None, 10_000, 100);
        insert_payment(&conn, "pay_2", None, 20_000, 100);
        insert_payment(&conn, "pay_3", None, 30_000, 90);

        let mut query = baseline_query(0, 1_000);
        query.limit = 2;
        let first = build_conversions_response(&conn, &query).expect("first page");
        assert_eq!(
            first
                .conversions
                .iter()
                .map(|payment| payment.provider_payment_id.as_str())
                .collect::<Vec<_>>(),
            vec!["pay_1", "pay_2"]
        );
        assert_eq!(first.next_cursor, Some("100:2".to_string()));

        query.cursor = Some(parse_cursor(first.next_cursor.as_deref().unwrap()).unwrap());
        let second = build_conversions_response(&conn, &query).expect("second page");
        assert_eq!(
            second
                .conversions
                .iter()
                .map(|payment| payment.provider_payment_id.as_str())
                .collect::<Vec<_>>(),
            vec!["pay_3"]
        );
        assert_eq!(second.next_cursor, None);
    }

    #[test]
    fn filters_payments_to_sessions_with_path_before_purchase() {
        let conn = open_migrated_db();
        let home = insert_path_with_url(&conn, "/", "example.com");
        let pricing = insert_path_with_url(&conn, "/pricing", "example.com");
        insert_landing(&conn, home, 1, 100, None, None);
        insert_pageview(&conn, 2, 150, 1, 100, pricing);
        insert_payment(&conn, "pay_before", Some(1), 10_000, 200);
        insert_landing(&conn, home, 3, 100, None, None);
        insert_pageview(&conn, 4, 250, 3, 100, pricing);
        insert_payment(&conn, "pay_after", Some(3), 20_000, 200);
        insert_landing(&conn, home, 5, 100, None, None);
        insert_payment(&conn, "pay_without_path", Some(5), 30_000, 200);

        let mut query = baseline_query(0, 1_000);
        query.path_id = Some(pricing);
        let response = build_conversions_response(&conn, &query).expect("response");

        assert_eq!(
            response
                .conversions
                .iter()
                .map(|payment| payment.provider_payment_id.as_str())
                .collect::<Vec<_>>(),
            vec!["pay_before"]
        );
    }

    #[test]
    fn filters_payments_by_landing_path() {
        let conn = open_migrated_db();
        let home = insert_path_with_url(&conn, "/", "example.com");
        let pricing = insert_path_with_url(&conn, "/pricing", "example.com");
        insert_landing(&conn, home, 1, 100, None, None);
        insert_payment(&conn, "pay_home", Some(1), 10_000, 200);
        insert_landing(&conn, pricing, 2, 100, None, None);
        insert_payment(&conn, "pay_pricing", Some(2), 20_000, 200);

        let mut query = baseline_query(0, 1_000);
        query.landing_path_id = Some(home);
        let response = build_conversions_response(&conn, &query).expect("response");

        assert_eq!(
            response
                .conversions
                .iter()
                .map(|payment| payment.provider_payment_id.as_str())
                .collect::<Vec<_>>(),
            vec!["pay_home"]
        );
    }

    #[test]
    fn excludes_payments_when_path_pageview_is_outside_range() {
        let conn = open_migrated_db();
        let home = insert_path_with_url(&conn, "/", "example.com");
        let pricing = insert_path_with_url(&conn, "/pricing", "example.com");
        insert_landing(&conn, home, 1, 50, None, None);
        insert_pageview(&conn, 2, 50, 1, 50, pricing);
        insert_payment(&conn, "pay_outside_window", Some(1), 10_000, 150);

        let mut query = baseline_query(100, 200);
        query.path_id = Some(pricing);
        let response = build_conversions_response(&conn, &query).expect("response");

        assert!(response.conversions.is_empty());
    }

    #[test]
    fn includes_path_payments_when_paid_at_is_outside_range() {
        let conn = open_migrated_db();
        let home = insert_path_with_url(&conn, "/", "example.com");
        let pricing = insert_path_with_url(&conn, "/pricing", "example.com");
        insert_landing(&conn, home, 1, 100, None, None);
        insert_pageview(&conn, 2, 150, 1, 100, pricing);
        // Pageview in [100, 200); payment after the window — still included for
        // path drilldowns (visit cohort, matching the Pages report).
        insert_payment(&conn, "pay_after_window", Some(1), 10_000, 250);

        let mut query = baseline_query(100, 200);
        query.path_id = Some(pricing);
        let response = build_conversions_response(&conn, &query).expect("response");

        assert_eq!(
            response
                .conversions
                .iter()
                .map(|payment| payment.provider_payment_id.as_str())
                .collect::<Vec<_>>(),
            vec!["pay_after_window"]
        );
    }

    #[test]
    fn excludes_landing_attribution_when_session_started_outside_range() {
        let conn = open_migrated_db();
        let home = insert_path_with_url(&conn, "/", "example.com");
        insert_landing(&conn, home, 1, 50, None, None);
        insert_payment(&conn, "pay_outside_window", Some(1), 10_000, 150);

        let mut query = baseline_query(100, 200);
        query.landing_path_id = Some(home);
        let response = build_conversions_response(&conn, &query).expect("response");

        assert!(response.conversions.is_empty());
    }

    #[test]
    fn includes_landing_payments_when_paid_at_is_outside_range() {
        let conn = open_migrated_db();
        let home = insert_path_with_url(&conn, "/", "example.com");
        insert_landing(&conn, home, 1, 150, None, None);
        insert_payment(&conn, "pay_after_window", Some(1), 10_000, 250);

        let mut query = baseline_query(100, 200);
        query.landing_path_id = Some(home);
        let response = build_conversions_response(&conn, &query).expect("response");

        assert_eq!(
            response
                .conversions
                .iter()
                .map(|payment| payment.provider_payment_id.as_str())
                .collect::<Vec<_>>(),
            vec!["pay_after_window"]
        );
    }

    #[test]
    fn parses_optional_revenue_source_filters() {
        let referrer =
            parse_payments_query("/v2/read/projects/1/conversions?start=1&end=100&referrer_id=12")
                .expect("referrer query");
        assert_eq!(
            referrer.revenue_source,
            Some(RevenueSourceFilter::Referrer(12))
        );

        let campaign =
            parse_payments_query("/v2/read/projects/1/conversions?start=1&end=100&campaign_id=34")
                .expect("campaign query");
        assert_eq!(
            campaign.revenue_source,
            Some(RevenueSourceFilter::Campaign(34))
        );

        let direct =
            parse_payments_query("/v2/read/projects/1/conversions?start=1&end=100&direct=1")
                .expect("direct query");
        assert_eq!(direct.revenue_source, Some(RevenueSourceFilter::Direct));
    }

    #[test]
    fn rejects_conflicting_revenue_source_filters() {
        assert!(
            parse_payments_query(
                "/v2/read/projects/1/conversions?start=1&end=100&referrer_id=1&campaign_id=2"
            )
            .is_err()
        );
        assert!(
            parse_payments_query(
                "/v2/read/projects/1/conversions?start=1&end=100&direct=true&referrer_id=1"
            )
            .is_err()
        );
    }

    #[test]
    fn filters_payments_by_referrer_source() {
        let conn = open_migrated_db();
        let home = insert_path(&conn);
        let google = insert_referrer(&conn, "google.com");
        let twitter = insert_referrer(&conn, "twitter.com");
        insert_landing(&conn, home, 1, 100, Some(google), None);
        insert_payment(&conn, "pay_google", Some(1), 10_000, 200);
        insert_landing(&conn, home, 2, 100, Some(twitter), None);
        insert_payment(&conn, "pay_twitter", Some(2), 20_000, 200);

        let mut query = baseline_query(0, 1_000);
        query.revenue_source = Some(RevenueSourceFilter::Referrer(google));
        let response = build_conversions_response(&conn, &query).expect("response");

        assert_eq!(
            response
                .conversions
                .iter()
                .map(|payment| payment.provider_payment_id.as_str())
                .collect::<Vec<_>>(),
            vec!["pay_google"]
        );
    }

    #[test]
    fn filters_payments_by_campaign_source() {
        let conn = open_migrated_db();
        let home = insert_path(&conn);
        let google = insert_referrer(&conn, "google.com");
        let campaign = insert_campaign(&conn);
        insert_landing(&conn, home, 1, 100, Some(google), Some(campaign));
        insert_payment(&conn, "pay_campaign", Some(1), 10_000, 200);
        insert_landing(&conn, home, 2, 100, Some(google), None);
        insert_payment(&conn, "pay_referrer", Some(2), 20_000, 200);

        let mut query = baseline_query(0, 1_000);
        query.revenue_source = Some(RevenueSourceFilter::Campaign(campaign));
        let response = build_conversions_response(&conn, &query).expect("response");

        assert_eq!(
            response
                .conversions
                .iter()
                .map(|payment| payment.provider_payment_id.as_str())
                .collect::<Vec<_>>(),
            vec!["pay_campaign"]
        );
    }

    #[test]
    fn filters_payments_by_direct_source() {
        let conn = open_migrated_db();
        let home = insert_path(&conn);
        let google = insert_referrer(&conn, "google.com");
        insert_landing(&conn, home, 1, 100, None, None);
        insert_payment(&conn, "pay_direct", Some(1), 10_000, 200);
        insert_landing(&conn, home, 2, 100, Some(google), None);
        insert_payment(&conn, "pay_referrer", Some(2), 20_000, 200);

        let mut query = baseline_query(0, 1_000);
        query.revenue_source = Some(RevenueSourceFilter::Direct);
        let response = build_conversions_response(&conn, &query).expect("response");

        assert_eq!(
            response
                .conversions
                .iter()
                .map(|payment| payment.provider_payment_id.as_str())
                .collect::<Vec<_>>(),
            vec!["pay_direct"]
        );
    }

    #[test]
    fn excludes_revenue_source_payments_when_landing_started_outside_range() {
        let conn = open_migrated_db();
        let home = insert_path(&conn);
        let google = insert_referrer(&conn, "google.com");
        insert_landing(&conn, home, 1, 50, Some(google), None);
        insert_payment(&conn, "pay_outside_window", Some(1), 10_000, 150);

        let mut query = baseline_query(100, 200);
        query.revenue_source = Some(RevenueSourceFilter::Referrer(google));
        let response = build_conversions_response(&conn, &query).expect("response");

        assert!(response.conversions.is_empty());
    }

    #[test]
    fn campaign_display_label_prefers_display_name() {
        let label = campaign_display_label(&PaymentCampaign {
            id: 1,
            source: "newsletter".to_string(),
            medium: "email".to_string(),
            campaign: "launch".to_string(),
            network: Some("google_ads".to_string()),
            display_name: Some("Spring Launch".to_string()),
            first_seen_at: 1,
            deleted_at: None,
        });
        assert_eq!(label, "Spring Launch");
    }

    #[test]
    fn campaign_display_label_joins_attribution_fields() {
        let label = campaign_display_label(&PaymentCampaign {
            id: 1,
            source: "newsletter".to_string(),
            medium: "cpc".to_string(),
            campaign: "launch".to_string(),
            network: Some("google_ads".to_string()),
            display_name: None,
            first_seen_at: 1,
            deleted_at: None,
        });
        assert_eq!(
            label,
            "ads=google_ads, source=newsletter, medium=cpc, campaign=launch"
        );
    }

    #[test]
    fn campaign_display_label_omits_generic_network() {
        let label = campaign_display_label(&PaymentCampaign {
            id: 1,
            source: "newsletter".to_string(),
            medium: "cpc".to_string(),
            campaign: "launch".to_string(),
            network: Some("generic".to_string()),
            display_name: None,
            first_seen_at: 1,
            deleted_at: None,
        });
        assert_eq!(label, "source=newsletter, medium=cpc, campaign=launch");
    }

    #[test]
    fn campaign_display_label_falls_back_when_empty() {
        let label = campaign_display_label(&PaymentCampaign {
            id: 1,
            source: String::new(),
            medium: String::new(),
            campaign: String::new(),
            network: None,
            display_name: None,
            first_seen_at: 1,
            deleted_at: None,
        });
        assert_eq!(label, UNNAMED_CAMPAIGN_LABEL);
    }
}
