use std::sync::Arc;

use rusqlite::params;
use serde::Serialize;
use serde_json::Value;

use crate::auth::{AuthError, map_row_error};
use crate::authorize::AuthorizedProject;
use crate::db::{ProjectConnection, open_project_database_read};
use crate::payments::{PaymentEntry, campaign_display_label, fetch_payment_entry_by_id};
use crate::query_params::parse_positive_id;
use crate::referrer::{ReferrerType, SourceGroup, source_group_for_landing};
use crate::server_context::ServerContext;
use crate::util::NANOS_PER_SECOND;

#[derive(Debug, Serialize)]
pub(crate) struct PaymentJourneyResponse {
    pub(crate) payment: PaymentEntry,
    pub(crate) timeline: Vec<PaymentJourneyAction>,
}

#[derive(Debug, Serialize)]
pub(crate) struct PaymentJourneyAction {
    pub(crate) kind: String,
    pub(crate) occurred_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) subtitle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) hostname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) path_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) duration_seconds: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) scrolled_percent: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) metadata: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) source_group: Option<SourceGroup>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) goal_id: Option<i64>,
}

pub(crate) struct PaymentJourneyQuery {
    pub(crate) payment_id: i64,
}

pub(crate) fn parse_payment_journey_query(
    target: &str,
) -> Result<PaymentJourneyQuery, &'static str> {
    let value = crate::http::query_param(target, "payment_id").ok_or("payment_id is required")?;
    Ok(PaymentJourneyQuery {
        payment_id: parse_positive_id(value, "invalid payment_id")?,
    })
}

/// The payment's journey, or not found when it was paid before `oldest`, the
/// first second the plan's retention shows.
pub(crate) fn payment_journey(
    server_context: Arc<ServerContext>,
    project: &AuthorizedProject,
    query: PaymentJourneyQuery,
    oldest: Option<i64>,
) -> Result<PaymentJourneyResponse, AuthError> {
    let journey = fetch_payment_journey(server_context, project.id(), &query)?;
    if oldest.is_some_and(|oldest| journey.payment.paid_at < oldest) {
        return Err(AuthError::InvalidRequest("payment not found"));
    }
    Ok(journey)
}

fn fetch_payment_journey(
    server_context: Arc<ServerContext>,
    project_id: &str,
    query: &PaymentJourneyQuery,
) -> Result<PaymentJourneyResponse, AuthError> {
    let conn = open_project_database_read(server_context.as_ref(), project_id)
        .map_err(|error| AuthError::Db(error.to_string()))?;
    build_payment_journey_response(&conn, query.payment_id)
}

pub(crate) fn build_payment_journey_response(
    conn: &ProjectConnection,
    payment_id: i64,
) -> Result<PaymentJourneyResponse, AuthError> {
    let payment = fetch_payment_entry_by_id(conn, payment_id)?
        .ok_or(AuthError::InvalidRequest("payment not found"))?;
    let mut timeline = Vec::new();
    if let Some(source) = build_source_action(&payment) {
        timeline.push(source);
    }
    if let Some(visitor_timestamp_id) = payment.visitor_timestamp_id {
        timeline.extend(fetch_pageview_actions(conn, visitor_timestamp_id)?);
        timeline.extend(fetch_event_actions(conn, visitor_timestamp_id)?);
    }
    timeline.push(PaymentJourneyAction {
        kind: "purchase".to_string(),
        occurred_at: payment.paid_at,
        title: Some("Purchase".to_string()),
        name: None,
        subtitle: Some(format!(
            "{} {}",
            payment.currency,
            format_thousandths_amount(payment.amount)
        )),
        hostname: None,
        path: None,
        path_id: None,
        duration_seconds: None,
        scrolled_percent: None,
        metadata: payment.metadata.clone(),
        source_group: None,
        goal_id: None,
    });
    sort_journey_timeline(&mut timeline);
    Ok(PaymentJourneyResponse { payment, timeline })
}

fn build_source_action(payment: &PaymentEntry) -> Option<PaymentJourneyAction> {
    let landing = payment.landing.as_ref()?;
    Some(source_action_from_parts(
        landing.viewed_at,
        landing.campaign.as_ref().map(campaign_display_label),
        landing
            .referrer
            .as_ref()
            .map(|referrer| referrer.referrer.clone()),
        landing.referrer_type,
    ))
}

pub(crate) fn source_action_from_parts(
    viewed_at: i64,
    campaign_label: Option<String>,
    referrer: Option<String>,
    referrer_type: Option<ReferrerType>,
) -> PaymentJourneyAction {
    let has_campaign = campaign_label.is_some();
    let has_referrer = referrer.is_some();
    let subtitle = campaign_label
        .or(referrer)
        .unwrap_or_else(|| "Direct".to_string());
    PaymentJourneyAction {
        kind: "source".to_string(),
        occurred_at: viewed_at,
        title: None,
        name: None,
        subtitle: Some(subtitle),
        hostname: None,
        path: None,
        path_id: None,
        duration_seconds: None,
        scrolled_percent: None,
        metadata: None,
        source_group: Some(source_group_for_landing(
            has_campaign,
            has_referrer,
            referrer_type,
        )),
        goal_id: None,
    }
}

pub(crate) fn fetch_pageview_actions(
    conn: &ProjectConnection,
    visitor_timestamp_id: i64,
) -> Result<Vec<PaymentJourneyAction>, AuthError> {
    conn.with_query(
        "SELECT
            pv.timestamp_id / ?2 AS occurred_at,
            paths.hostname,
            paths.path,
            NULLIF((
                SELECT json_group_object(m.key, m.value)
                FROM metadata m
                WHERE m.page_timestamp_id = pv.timestamp_id
            ), '{}') AS metadata,
            pv.duration_seconds,
            pv.scrolled_percent,
            pv.path_id,
            funnel_steps.id AS goal_id
         FROM pageviews pv
         JOIN paths ON paths.id = pv.path_id
         LEFT JOIN (
            SELECT funnel_steps.path_id, MIN(funnel_steps.id) AS id
            FROM funnel_steps
            JOIN funnels f ON f.id = funnel_steps.funnel_id AND f.deleted_at IS NULL
            WHERE funnel_steps.path_id IS NOT NULL
            GROUP BY funnel_steps.path_id
         ) funnel_steps ON funnel_steps.path_id = pv.path_id
         WHERE pv.visitor_timestamp_id = ?1",
        params![visitor_timestamp_id, NANOS_PER_SECOND],
        |mut rows| {
            let mut out = Vec::new();
            while let Some(row) = rows.next()? {
                let hostname = row.get::<_, String>(1)?;
                let path = row.get::<_, String>(2)?;
                out.push(PaymentJourneyAction {
                    kind: "pageview".to_string(),
                    occurred_at: row.get(0)?,
                    title: Some("Page view".to_string()),
                    name: None,
                    subtitle: Some(format!("{hostname}{path}")),
                    hostname: Some(hostname),
                    path: Some(path),
                    path_id: Some(row.get(6)?),
                    duration_seconds: row.get(4)?,
                    scrolled_percent: row.get(5)?,
                    metadata: parse_optional_json(row.get(3)?),
                    source_group: None,
                    goal_id: row.get(7)?,
                });
            }
            Ok(out)
        },
    )
    .map_err(map_row_error)
}

pub(crate) fn fetch_event_actions(
    conn: &ProjectConnection,
    visitor_timestamp_id: i64,
) -> Result<Vec<PaymentJourneyAction>, AuthError> {
    conn.with_query(
        "SELECT
            events.timestamp_id / ?2 AS occurred_at,
            event_names.name,
            json(events.metadata),
            paths.hostname,
            paths.path,
            funnel_steps.id AS goal_id
         FROM events
         JOIN event_names ON event_names.id = events.event_name_id
         JOIN pageviews pv ON pv.timestamp_id = events.page_timestamp_id
         JOIN paths ON paths.id = pv.path_id
         LEFT JOIN (
            SELECT funnel_steps.event_name_id, MIN(funnel_steps.id) AS id
            FROM funnel_steps
            JOIN funnels f ON f.id = funnel_steps.funnel_id AND f.deleted_at IS NULL
            WHERE funnel_steps.event_name_id IS NOT NULL
            GROUP BY funnel_steps.event_name_id
         ) funnel_steps ON funnel_steps.event_name_id = events.event_name_id
         WHERE pv.visitor_timestamp_id = ?1",
        params![visitor_timestamp_id, NANOS_PER_SECOND],
        |mut rows| {
            let mut out = Vec::new();
            while let Some(row) = rows.next()? {
                let name: String = row.get(1)?;
                let hostname = row.get::<_, String>(3)?;
                let path = row.get::<_, String>(4)?;
                out.push(PaymentJourneyAction {
                    kind: "event".to_string(),
                    occurred_at: row.get(0)?,
                    title: None,
                    name: Some(name),
                    subtitle: Some(format!("{hostname}{path}")),
                    hostname: Some(hostname),
                    path: Some(path),
                    path_id: None,
                    duration_seconds: None,
                    scrolled_percent: None,
                    metadata: parse_optional_json(row.get(2)?),
                    source_group: None,
                    goal_id: row.get(5)?,
                });
            }
            Ok(out)
        },
    )
    .map_err(map_row_error)
}

pub(crate) fn sort_journey_timeline(timeline: &mut [PaymentJourneyAction]) {
    timeline.sort_by(|left, right| {
        left.occurred_at
            .cmp(&right.occurred_at)
            .then_with(|| journey_kind_rank(&left.kind).cmp(&journey_kind_rank(&right.kind)))
    });
}

fn journey_kind_rank(kind: &str) -> u8 {
    match kind {
        "source" => 0,
        "pageview" => 1,
        "event" => 2,
        "purchase" => 3,
        _ => 4,
    }
}

fn format_thousandths_amount(amount: i64) -> String {
    let major = amount as f64 / 1_000.0;
    format!("{major:.2}")
}

fn parse_optional_json(value: Option<String>) -> Option<Value> {
    value.and_then(|value| serde_json::from_str(&value).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::open_migrated_db;

    use crate::test_utils::{
        PaymentFixture, insert_payment as insert_test_payment, visitor_timestamp_id_for_page_id,
    };
    use rusqlite::params;

    fn page_id(seed: u8) -> [u8; 16] {
        [seed; 16]
    }

    fn timestamp_id(seed: u8, viewed_at: i64) -> i64 {
        viewed_at * NANOS_PER_SECOND + i64::from(seed)
    }

    fn insert_path(conn: &ProjectConnection) -> i64 {
        conn.execute(
            "INSERT INTO paths (path, hostname) VALUES ('/', 'example.com')",
            [],
        )
        .expect("insert path");
        conn.query_row("SELECT id FROM paths ORDER BY id DESC LIMIT 1", [], |row| {
            row.get::<_, i64>(0)
        })
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
        insert_test_payment(conn, payment);
    }

    fn fetch_payment_id(conn: &ProjectConnection, provider_payment_id: &str) -> i64 {
        conn.query_row(
            "SELECT id FROM payments WHERE provider_payment_id = ?1",
            params![provider_payment_id],
            |row| row.get::<_, i64>(0),
        )
        .expect("payment id")
    }

    fn insert_pageview(
        conn: &ProjectConnection,
        path_id: i64,
        visitor_seed: u8,
        page_seed: u8,
        viewed_at: i64,
    ) {
        let visitor_timestamp_id = timestamp_id(visitor_seed, 100);
        let page_timestamp_id = timestamp_id(page_seed, viewed_at);
        conn.execute(
            "INSERT INTO pageviews (
                timestamp_id, visitor_timestamp_id, previous_timestamp_id, path_id,
                duration_seconds, scrolled_percent
             ) VALUES (?1, ?2, ?2, ?3, 8, 90)",
            params![page_timestamp_id, visitor_timestamp_id, path_id],
        )
        .expect("insert pageview");
        conn.execute(
            "INSERT INTO metadata (visitor_timestamp_id, key, value, page_timestamp_id)
             VALUES (?1, 'ref', 'detail', ?2), (?1, 'section', 'pricing', ?2)",
            params![visitor_timestamp_id, page_timestamp_id],
        )
        .expect("insert metadata");
    }

    fn insert_event(
        conn: &ProjectConnection,
        page_seed: u8,
        page_viewed_at: i64,
        event_seed: u8,
        occurred_at: i64,
    ) -> i64 {
        conn.execute(
            "INSERT INTO event_names (name, deleted_at) VALUES ('cta_clicked', NULL)",
            [],
        )
        .expect("insert event name");
        let event_name_id = conn
            .query_row(
                "SELECT id FROM event_names WHERE name = 'cta_clicked'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("event name id");
        let event_timestamp_id = timestamp_id(event_seed, occurred_at);
        conn.execute(
            "INSERT INTO events (timestamp_id, event_name_id, page_timestamp_id, metadata)
             VALUES (?1, ?2, ?3, jsonb('{\"button\":\"buy\"}'))",
            params![
                event_timestamp_id,
                event_name_id,
                timestamp_id(page_seed, page_viewed_at),
            ],
        )
        .expect("insert event");
        event_name_id
    }

    fn insert_event_goal(conn: &ProjectConnection, path_id: i64, event_name_id: i64) -> i64 {
        conn.execute("INSERT INTO funnels (name) VALUES ('test')", [])
            .expect("insert funnel");
        let funnel_id = conn
            .query_row(
                "SELECT id FROM funnels ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("funnel id");
        conn.execute(
            "INSERT INTO funnel_steps (funnel_id, step_type, path_id, event_name_id, is_landing, description, step_order)
             VALUES (?1, 'pageview', ?2, ?3, 0, 'signup', 0)",
            params![funnel_id, path_id, event_name_id],
        )
        .expect("insert funnel step");
        conn.query_row(
            "SELECT id FROM funnel_steps WHERE event_name_id = ?1",
            params![event_name_id],
            |row| row.get::<_, i64>(0),
        )
        .expect("funnel step id")
    }

    #[test]
    fn returns_payment_journey_timeline() {
        let conn = open_migrated_db();
        let path_id = insert_path(&conn);
        let referrer = insert_referrer(&conn, "google.com/search");
        insert_landing(&conn, path_id, 1, 100, Some(referrer), None);
        insert_pageview(&conn, path_id, 1, 2, 130);
        let event_name_id = insert_event(&conn, 2, 130, 3, 140);
        let goal_id = insert_event_goal(&conn, path_id, event_name_id);
        insert_payment(&conn, "pay_1", Some(1), 25_000, 200);
        let payment_id = fetch_payment_id(&conn, "pay_1");

        let response = build_payment_journey_response(&conn, payment_id).expect("journey");

        assert_eq!(response.payment.provider_payment_id, "pay_1");
        assert_eq!(
            response
                .timeline
                .iter()
                .map(|action| action.kind.as_str())
                .collect::<Vec<_>>(),
            vec!["source", "pageview", "pageview", "event", "purchase"]
        );
        assert_eq!(
            response
                .timeline
                .iter()
                .map(|action| action.occurred_at)
                .collect::<Vec<_>>(),
            vec![100, 100, 130, 140, 200]
        );
        assert_eq!(response.timeline[0].source_group, Some(SourceGroup::Search));
        assert_eq!(response.timeline[1].path_id, Some(path_id));
        assert_eq!(response.timeline[2].path_id, Some(path_id));
        assert_eq!(response.timeline[3].name.as_deref(), Some("cta_clicked"));
        assert_eq!(response.timeline[3].goal_id, Some(goal_id));
        assert_eq!(
            response.timeline[3]
                .metadata
                .as_ref()
                .and_then(|value| value.get("button")),
            Some(&Value::String("buy".to_string()))
        );
    }
}
