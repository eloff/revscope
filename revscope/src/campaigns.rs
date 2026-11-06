//! Campaign catalog: auto-detected UTM campaigns plus user rules that assign
//! unclassified landings by a stored query parameter.

use std::sync::Arc;

use fxhash::FxHashMap;
use rusqlite::OptionalExtension;
use rusqlite::types::ToSql;
use rusqlite::{params, params_from_iter};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::auth::{AuthError, map_row_error};
use crate::authorize::{AuthorizedProject, WritableProject};
use crate::campaign_spend::{self, CampaignSpendView};
use crate::comparison::{self, Change};
use crate::db::{ProjectConnection, open_project_database_read, open_project_database_write};
use crate::metrics::{self, WilsonInterval};
use crate::page_revenue::page_revenue_cte;
use crate::query_params::parse_date_range_query;
use crate::segment_filter::{SegmentDimension, SegmentFilters, parse_segment_filters};
use crate::server_context::ServerContext;
use crate::timezone::{self, ProjectTimezone};
use crate::util::{NANOS_PER_SECOND, nanoseconds_since_epoch, seconds_since_epoch};

const MAX_NAME_LEN: usize = 80;
const MAX_PARAM_KEY_LEN: usize = 64;
const MAX_PARAM_VALUE_LEN: usize = 200;
const MAX_RULES: usize = 8;
const TOP_BREAKDOWN: usize = 5;
const TOP_SUGGESTIONS: i64 = 8;

/// A stored matcher. Several rules on one campaign are alternatives.
#[derive(Clone, Debug)]
pub(crate) struct CampaignRuleMatch {
    pub(crate) campaign_id: i64,
    pub(crate) param_key: String,
    pub(crate) param_value: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct CampaignRuleInput {
    #[serde(default)]
    pub(crate) param_key: String,
    #[serde(default)]
    pub(crate) param_value: String,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct CampaignRuleView {
    pub(crate) id: i64,
    pub(crate) param_key: String,
    pub(crate) param_value: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct CampaignCount {
    pub(crate) label: String,
    pub(crate) visitors: i64,
}

#[derive(Debug, Serialize)]
pub(crate) struct CampaignPathCount {
    pub(crate) id: i64,
    pub(crate) path: String,
    pub(crate) count: i64,
}

#[derive(Debug, Serialize)]
pub(crate) struct CampaignListItem {
    pub(crate) id: i64,
    pub(crate) source: String,
    pub(crate) medium: String,
    pub(crate) campaign: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) network: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) display_name: Option<String>,
    pub(crate) first_seen_at: i64,
    pub(crate) rules: Vec<CampaignRuleView>,
    pub(crate) visitors: i64,
    pub(crate) converting_visitors: i64,
    pub(crate) revenue: i64,
    pub(crate) conversion_rate: f64,
    /// 95% Wilson interval for the conversion rate; `None` without visitors.
    pub(crate) conversion_ci: Option<WilsonInterval>,
    pub(crate) revenue_per_visitor: f64,
    /// Entered spend on finished days within the range.
    pub(crate) spend: i64,
    /// Revenue over spend; `None` without spend.
    pub(crate) roas: Option<f64>,
    /// (Revenue - spend) over spend; `None` without spend.
    pub(crate) roi: Option<f64>,
    pub(crate) spend_entries: Vec<CampaignSpendView>,
    pub(crate) visitors_change: Change,
    pub(crate) revenue_change: Change,
    pub(crate) conversion_rate_change: Change,
    pub(crate) referrers: Vec<CampaignCount>,
    pub(crate) landing_paths: Vec<CampaignPathCount>,
    pub(crate) countries: Vec<CampaignCount>,
    pub(crate) devices: Vec<CampaignCount>,
    pub(crate) contents: Vec<CampaignCount>,
    pub(crate) terms: Vec<CampaignCount>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CampaignSuggestion {
    /// Query parameter key: `source` or `ref`.
    pub(crate) kind: String,
    pub(crate) value: String,
    pub(crate) visitors: i64,
}

#[derive(Debug, Serialize)]
pub(crate) struct CampaignsResponse {
    pub(crate) campaigns: Vec<CampaignListItem>,
    pub(crate) suggestions: Vec<CampaignSuggestion>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CampaignPreviewResponse {
    /// Matching landings in the range that have no campaign yet.
    pub(crate) unclassified: i64,
    /// Unclassified matches with no referrer.
    pub(crate) direct: i64,
    /// Unclassified matches that have a referrer.
    pub(crate) referrer: i64,
    /// Matches that already belong to a campaign and will be left alone.
    pub(crate) already_classified: i64,
}

#[derive(Debug, Serialize)]
pub(crate) struct SaveCampaignResponse {
    pub(crate) id: i64,
    /// Landings newly assigned by this save.
    pub(crate) assigned: i64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SaveCampaignRequest {
    #[serde(default)]
    pub(crate) id: Option<i64>,
    pub(crate) name: String,
    pub(crate) rules: Vec<CampaignRuleInput>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CampaignPreviewRequest {
    pub(crate) rules: Vec<CampaignRuleInput>,
}

pub(crate) struct CampaignsQuery {
    pub(crate) start: i64,
    pub(crate) end: i64,
    pub(crate) segments: SegmentFilters,
}

pub(crate) struct DeleteCampaignQuery {
    pub(crate) id: i64,
}

#[derive(Clone, Copy, Default)]
struct Counts {
    visitors: i64,
    converting_visitors: i64,
    revenue: i64,
}

#[derive(Default)]
struct Breakdowns {
    referrers: FxHashMap<String, i64>,
    paths: FxHashMap<i64, i64>,
    countries: FxHashMap<String, i64>,
    devices: FxHashMap<String, i64>,
    contents: FxHashMap<String, i64>,
    terms: FxHashMap<String, i64>,
}

struct CatalogRow {
    id: i64,
    source: String,
    medium: String,
    campaign: String,
    network: Option<String>,
    display_name: Option<String>,
    first_seen_at: i64,
    rules: Vec<CampaignRuleView>,
}

pub(crate) fn parse_campaigns_query(target: &str) -> Result<CampaignsQuery, &'static str> {
    let (start, end) = parse_date_range_query(target)?;
    Ok(CampaignsQuery {
        start,
        end,
        segments: parse_segment_filters(target),
    })
}

pub(crate) fn parse_delete_campaign_query(
    target: &str,
) -> Result<DeleteCampaignQuery, &'static str> {
    let id = crate::http::query_param(target, "id")
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|value| *value > 0)
        .ok_or("id is required")?;
    Ok(DeleteCampaignQuery { id })
}

pub(crate) fn normalize_save_request(
    mut request: SaveCampaignRequest,
) -> Result<SaveCampaignRequest, &'static str> {
    request.name = normalize_display_name(&request.name)?;
    if let Some(id) = request.id
        && id <= 0
    {
        return Err("id is required");
    }
    if request.rules.is_empty() {
        if request.id.is_none() {
            return Err("at least one rule is required");
        }
    } else {
        request.rules = normalize_rules(request.rules)?;
    }
    Ok(request)
}

pub(crate) fn normalize_preview_request(
    mut request: CampaignPreviewRequest,
) -> Result<CampaignPreviewRequest, &'static str> {
    request.rules = normalize_rules(request.rules)?;
    Ok(request)
}

pub(crate) fn campaigns(
    server_context: Arc<ServerContext>,
    project: &AuthorizedProject,
    query: CampaignsQuery,
) -> Result<CampaignsResponse, AuthError> {
    let timezone = timezone::project_timezone(server_context.clone(), project.key());
    let conn = open_project_database_read(server_context.as_ref(), project.id())
        .map_err(|error| AuthError::Db(error.to_string()))?;
    build_campaigns_response_at(&conn, &query, &timezone, seconds_since_epoch() as i64)
}

pub(crate) fn campaign_preview(
    server_context: Arc<ServerContext>,
    project: &AuthorizedProject,
    request: CampaignPreviewRequest,
    query: &CampaignsQuery,
) -> Result<CampaignPreviewResponse, AuthError> {
    let conn = open_project_database_read(server_context.as_ref(), project.id())
        .map_err(|error| AuthError::Db(error.to_string()))?;
    preview_rules(&conn, &request.rules, query)
}

pub(crate) fn save_campaign(
    server_context: Arc<ServerContext>,
    project: &WritableProject,
    request: SaveCampaignRequest,
) -> Result<SaveCampaignResponse, AuthError> {
    let conn = open_project_database_write(server_context.as_ref(), project.id())
        .map_err(|error| AuthError::Db(error.to_string()))?;
    save_campaign_on(&conn, &request)
}

pub(crate) fn delete_campaign(
    server_context: Arc<ServerContext>,
    project: &WritableProject,
    query: DeleteCampaignQuery,
) -> Result<(), AuthError> {
    let conn = open_project_database_write(server_context.as_ref(), project.id())
        .map_err(|error| AuthError::Db(error.to_string()))?;
    let updated = conn
        .execute(
            "UPDATE campaigns SET deleted_at = unixepoch()
             WHERE id = ?1 AND deleted_at IS NULL",
            params![query.id],
        )
        .map_err(map_row_error)?;
    if updated == 0 {
        return Err(AuthError::InvalidRequest("campaign not found"));
    }
    Ok(())
}

/// Active rules for non-deleted campaigns, ordered so the lowest campaign id
/// wins when a landing matches more than one.
pub(crate) fn load_active_campaign_rules(
    conn: &ProjectConnection,
) -> Result<Vec<CampaignRuleMatch>, rusqlite::Error> {
    conn.with_query(
        "SELECT rule.campaign_id, rule.param_key, rule.param_value
         FROM campaign_rules rule
         JOIN campaigns c ON c.id = rule.campaign_id
         WHERE c.deleted_at IS NULL
         ORDER BY rule.campaign_id, rule.id",
        [],
        |mut rows| {
            let mut rules = Vec::new();
            while let Some(row) = rows.next()? {
                rules.push(CampaignRuleMatch {
                    campaign_id: row.get(0)?,
                    param_key: row.get(1)?,
                    param_value: row.get(2)?,
                });
            }
            Ok(rules)
        },
    )
}

/// First campaign with a rule matching one of this landing's query parameters.
pub(crate) fn matching_campaign_id(
    rules: &[CampaignRuleMatch],
    params: Option<&serde_json::Map<String, Value>>,
) -> Option<i64> {
    let params = params?;
    rules.iter().find_map(|rule| {
        if rule_matches(rule, params) {
            Some(rule.campaign_id)
        } else {
            None
        }
    })
}

fn rule_matches(rule: &CampaignRuleMatch, params: &serde_json::Map<String, Value>) -> bool {
    params.iter().any(|(param_key, param_value)| {
        param_key.eq_ignore_ascii_case(&rule.param_key)
            && param_value
                .as_str()
                .is_some_and(|text| text.eq_ignore_ascii_case(&rule.param_value))
    })
}

fn build_campaigns_response_at(
    conn: &ProjectConnection,
    query: &CampaignsQuery,
    timezone: &ProjectTimezone,
    now: i64,
) -> Result<CampaignsResponse, AuthError> {
    let previous_start = comparison::previous_period_start(query.start);
    let factor = comparison::normalization_factor(query.start, query.end, now);
    let catalog = load_catalog(conn)?;
    let mut spend =
        campaign_spend::load_campaign_spend(conn, timezone, query.start, query.end, now)?;
    let (mut current, path_ids) =
        aggregate_landings(conn, query.start, query.end, &query.segments, true)?;
    let (previous, _) =
        aggregate_landings(conn, previous_start, query.start, &query.segments, false)?;
    let paths = load_paths(conn, &path_ids)?;
    attach_metadata_breakdowns(conn, query.start, query.end, &query.segments, &mut current)?;

    let mut campaigns = Vec::with_capacity(catalog.len());
    for row in catalog {
        let counts = current
            .get(&row.id)
            .map(|bucket| bucket.counts)
            .unwrap_or_default();
        let previous_counts = previous.get(&row.id).map(|bucket| bucket.counts);
        let visitors = counts.visitors as f64;
        let converting = counts.converting_visitors as f64;
        let revenue = counts.revenue as f64;
        let conversion_rate = metrics::conversion_rate(converting, visitors);
        let revenue_per_visitor = metrics::revenue_per_visitor(revenue, visitors);
        let breakdowns = current.get(&row.id);
        let spend = spend.remove(&row.id).unwrap_or_default();
        campaigns.push(CampaignListItem {
            id: row.id,
            source: row.source,
            medium: row.medium,
            campaign: row.campaign,
            network: row.network,
            display_name: row.display_name,
            first_seen_at: row.first_seen_at,
            rules: row.rules,
            visitors: counts.visitors,
            converting_visitors: counts.converting_visitors,
            revenue: counts.revenue,
            conversion_rate,
            conversion_ci: metrics::conversion_rate_wilson_interval(converting, visitors),
            revenue_per_visitor,
            spend: spend.spend,
            roas: campaign_spend::roas(counts.revenue, spend.spend),
            roi: campaign_spend::roi(counts.revenue, spend.spend),
            spend_entries: spend.entries,
            visitors_change: Change::breakdown_absolute(
                visitors,
                previous_counts.map(|counts| counts.visitors as f64),
                factor,
            ),
            revenue_change: Change::breakdown_absolute(
                revenue,
                previous_counts.map(|counts| counts.revenue as f64),
                factor,
            ),
            conversion_rate_change: Change::breakdown_rate(
                conversion_rate,
                previous_counts.map(|counts| {
                    metrics::conversion_rate(
                        counts.converting_visitors as f64,
                        counts.visitors as f64,
                    )
                }),
            ),
            referrers: breakdowns
                .map(|bucket| top_labels(&bucket.breakdowns.referrers))
                .unwrap_or_default(),
            landing_paths: breakdowns
                .map(|bucket| top_paths(&bucket.breakdowns.paths, &paths))
                .unwrap_or_default(),
            countries: breakdowns
                .map(|bucket| top_labels(&bucket.breakdowns.countries))
                .unwrap_or_default(),
            devices: breakdowns
                .map(|bucket| top_labels(&bucket.breakdowns.devices))
                .unwrap_or_default(),
            contents: breakdowns
                .map(|bucket| top_labels(&bucket.breakdowns.contents))
                .unwrap_or_default(),
            terms: breakdowns
                .map(|bucket| top_labels(&bucket.breakdowns.terms))
                .unwrap_or_default(),
        });
    }

    campaigns.sort_by(|left, right| {
        right
            .revenue
            .cmp(&left.revenue)
            .then(right.visitors.cmp(&left.visitors))
            .then_with(|| campaign_sort_label(left).cmp(&campaign_sort_label(right)))
    });

    let suggestions = load_suggestions(conn, query)?;
    Ok(CampaignsResponse {
        campaigns,
        suggestions,
    })
}

fn campaign_sort_label(item: &CampaignListItem) -> String {
    item.display_name
        .clone()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| item.campaign.clone())
}

struct LandingBucket {
    counts: Counts,
    breakdowns: Breakdowns,
}

fn aggregate_landings(
    conn: &ProjectConnection,
    start: i64,
    end: i64,
    segments: &SegmentFilters,
    with_breakdowns: bool,
) -> Result<(FxHashMap<i64, LandingBucket>, Vec<i64>), AuthError> {
    let country_filter = segments.country_in_clause("l.country", 3);
    let session_filter =
        segments.session_filter_clause("l.visitor_timestamp_id", Some(SegmentDimension::Source));
    let page_revenue = page_revenue_cte(1, 2, true);
    let sql = format!(
        "WITH {page_revenue}
         SELECT
             l.campaign,
             COALESCE(rev.amount, 0),
             r.referrer,
             pv.path_id,
             l.country,
             l.device
         FROM landings l
         JOIN campaigns c ON c.id = l.campaign AND c.deleted_at IS NULL
         LEFT JOIN page_revenue rev ON rev.visitor_timestamp_id = l.visitor_timestamp_id
         LEFT JOIN referrers r ON r.id = l.referrer
         LEFT JOIN pageviews pv ON pv.timestamp_id = l.visitor_timestamp_id
         WHERE l.campaign IS NOT NULL
           AND l.visitor_timestamp_id >= ?1
           AND l.visitor_timestamp_id < ?2{country_filter}{session_filter}"
    );
    let start_ns = start * NANOS_PER_SECOND;
    let end_ns = end * NANOS_PER_SECOND;
    let mut sql_params: Vec<&dyn ToSql> = vec![&start_ns, &end_ns];
    segments.append_country_params(&mut sql_params);
    conn.with_query(&sql, params_from_iter(sql_params), |mut rows| {
        let mut buckets: FxHashMap<i64, LandingBucket> = FxHashMap::default();
        let mut path_ids = Vec::new();
        while let Some(row) = rows.next()? {
            let campaign_id: i64 = row.get(0)?;
            let amount: i64 = row.get(1)?;
            let bucket = buckets.entry(campaign_id).or_insert_with(|| LandingBucket {
                counts: Counts::default(),
                breakdowns: Breakdowns::default(),
            });
            bucket.counts.visitors += 1;
            if amount > 0 {
                bucket.counts.converting_visitors += 1;
                bucket.counts.revenue += amount;
            }
            if with_breakdowns {
                let referrer: Option<String> = row.get(2)?;
                let path_id: Option<i64> = row.get(3)?;
                let country: Option<String> = row.get(4)?;
                let device: String = row.get(5)?;
                let referrer_label = referrer.unwrap_or_else(|| "(direct)".to_string());
                *bucket
                    .breakdowns
                    .referrers
                    .entry(referrer_label)
                    .or_default() += 1;
                if let Some(path_id) = path_id {
                    *bucket.breakdowns.paths.entry(path_id).or_default() += 1;
                    if !path_ids.contains(&path_id) {
                        path_ids.push(path_id);
                    }
                }
                if let Some(country) = country.filter(|value| !value.is_empty()) {
                    *bucket.breakdowns.countries.entry(country).or_default() += 1;
                }
                if !device.is_empty() {
                    *bucket.breakdowns.devices.entry(device).or_default() += 1;
                }
            }
        }
        Ok((buckets, path_ids))
    })
    .map_err(map_row_error)
}

fn attach_metadata_breakdowns(
    conn: &ProjectConnection,
    start: i64,
    end: i64,
    segments: &SegmentFilters,
    buckets: &mut FxHashMap<i64, LandingBucket>,
) -> Result<(), AuthError> {
    if buckets.is_empty() {
        return Ok(());
    }
    let country_filter = segments.country_in_clause("l.country", 3);
    let session_filter =
        segments.session_filter_clause("l.visitor_timestamp_id", Some(SegmentDimension::Source));
    let sql = format!(
        "SELECT l.campaign, m.key, m.value, COUNT(*)
         FROM landings l
         JOIN campaigns c ON c.id = l.campaign AND c.deleted_at IS NULL
         JOIN metadata m ON m.visitor_timestamp_id = l.visitor_timestamp_id
         WHERE l.campaign IS NOT NULL
           AND l.visitor_timestamp_id >= ?1
           AND l.visitor_timestamp_id < ?2
           AND m.key IN ('content', 'term'){country_filter}{session_filter}
         GROUP BY l.campaign, m.key, m.value"
    );
    let start_ns = start * NANOS_PER_SECOND;
    let end_ns = end * NANOS_PER_SECOND;
    let mut sql_params: Vec<&dyn ToSql> = vec![&start_ns, &end_ns];
    segments.append_country_params(&mut sql_params);
    let rows = conn
        .with_query(&sql, params_from_iter(sql_params), |mut rows| {
            let mut grouped = Vec::new();
            while let Some(row) = rows.next()? {
                grouped.push((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ));
            }
            Ok(grouped)
        })
        .map_err(map_row_error)?;
    for (campaign_id, key, value, count) in rows {
        let Some(bucket) = buckets.get_mut(&campaign_id) else {
            continue;
        };
        let target = if key == "content" {
            &mut bucket.breakdowns.contents
        } else {
            &mut bucket.breakdowns.terms
        };
        *target.entry(value).or_default() += count;
    }
    Ok(())
}

fn load_catalog(conn: &ProjectConnection) -> Result<Vec<CatalogRow>, AuthError> {
    let mut rows = conn
        .with_query(
            "SELECT id, source, medium, campaign, network, display_name, first_seen_at
             FROM campaigns
             WHERE deleted_at IS NULL",
            [],
            |mut rows| {
                let mut catalog = Vec::new();
                while let Some(row) = rows.next()? {
                    catalog.push(CatalogRow {
                        id: row.get(0)?,
                        source: row.get(1)?,
                        medium: row.get(2)?,
                        campaign: row.get(3)?,
                        network: row.get(4)?,
                        display_name: row.get(5)?,
                        first_seen_at: row.get(6)?,
                        rules: Vec::new(),
                    });
                }
                Ok(catalog)
            },
        )
        .map_err(map_row_error)?;
    let rules = conn
        .with_query(
            "SELECT id, campaign_id, param_key, param_value
             FROM campaign_rules
             ORDER BY campaign_id, id",
            [],
            |mut rows| {
                let mut grouped: FxHashMap<i64, Vec<CampaignRuleView>> = FxHashMap::default();
                while let Some(row) = rows.next()? {
                    let campaign_id: i64 = row.get(1)?;
                    grouped
                        .entry(campaign_id)
                        .or_default()
                        .push(CampaignRuleView {
                            id: row.get(0)?,
                            param_key: row.get(2)?,
                            param_value: row.get(3)?,
                        });
                }
                Ok(grouped)
            },
        )
        .map_err(map_row_error)?;
    for row in &mut rows {
        if let Some(rules) = rules.get(&row.id) {
            row.rules.clone_from(rules);
        }
    }
    Ok(rows)
}

fn load_paths(conn: &ProjectConnection, ids: &[i64]) -> Result<FxHashMap<i64, String>, AuthError> {
    if ids.is_empty() {
        return Ok(FxHashMap::default());
    }
    let mut ordered = ids.to_vec();
    ordered.sort_unstable();
    ordered.dedup();
    let placeholders = (1..=ordered.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!("SELECT id, path FROM paths WHERE id IN ({placeholders})");
    conn.with_query(&sql, params_from_iter(ordered.iter()), |mut rows| {
        let mut paths = FxHashMap::default();
        while let Some(row) = rows.next()? {
            paths.insert(row.get(0)?, row.get(1)?);
        }
        Ok(paths)
    })
    .map_err(map_row_error)
}

fn load_suggestions(
    conn: &ProjectConnection,
    query: &CampaignsQuery,
) -> Result<Vec<CampaignSuggestion>, AuthError> {
    let mut suggestions = Vec::new();
    suggestions.extend(suggest_metadata(conn, query, "source")?);
    suggestions.extend(suggest_metadata(conn, query, "ref")?);
    Ok(suggestions)
}

fn suggest_metadata(
    conn: &ProjectConnection,
    query: &CampaignsQuery,
    key: &str,
) -> Result<Vec<CampaignSuggestion>, AuthError> {
    let country_filter = query.segments.country_in_clause("l.country", 4);
    let session_filter = query
        .segments
        .session_filter_clause("l.visitor_timestamp_id", Some(SegmentDimension::Source));
    let sql = format!(
        "SELECT MIN(m.value), COUNT(*)
         FROM landings l
         JOIN metadata m ON m.visitor_timestamp_id = l.visitor_timestamp_id
         WHERE l.campaign IS NULL
           AND l.visitor_timestamp_id >= ?1
           AND l.visitor_timestamp_id < ?2
           AND COALESCE(l.referrer_type, '') NOT IN ('ai', 'search', 'social', 'affiliate')
           AND m.key = ?3
           AND m.value != ''{country_filter}{session_filter}
         GROUP BY m.value COLLATE NOCASE
         ORDER BY COUNT(*) DESC, MIN(m.value) ASC
         LIMIT {TOP_SUGGESTIONS}"
    );
    let start_ns = query.start * NANOS_PER_SECOND;
    let end_ns = query.end * NANOS_PER_SECOND;
    let mut sql_params: Vec<&dyn ToSql> = vec![&start_ns, &end_ns, &key];
    query.segments.append_country_params(&mut sql_params);
    let kind = key.to_string();
    conn.with_query(&sql, params_from_iter(sql_params), |mut rows| {
        let mut suggestions = Vec::new();
        while let Some(row) = rows.next()? {
            suggestions.push(CampaignSuggestion {
                kind: kind.clone(),
                value: row.get(0)?,
                visitors: row.get(1)?,
            });
        }
        Ok(suggestions)
    })
    .map_err(map_row_error)
}

fn top_labels(counts: &FxHashMap<String, i64>) -> Vec<CampaignCount> {
    let mut ranked: Vec<(&String, &i64)> = counts.iter().filter(|(_, count)| **count > 0).collect();
    ranked.sort_by(|left, right| right.1.cmp(left.1).then(left.0.cmp(right.0)));
    ranked.truncate(TOP_BREAKDOWN);
    ranked
        .into_iter()
        .map(|(label, visitors)| CampaignCount {
            label: label.clone(),
            visitors: *visitors,
        })
        .collect()
}

fn top_paths(
    counts: &FxHashMap<i64, i64>,
    paths: &FxHashMap<i64, String>,
) -> Vec<CampaignPathCount> {
    let mut ranked: Vec<(&i64, &i64)> = counts.iter().collect();
    ranked.sort_by(|left, right| right.1.cmp(left.1).then(left.0.cmp(right.0)));
    ranked.truncate(TOP_BREAKDOWN);
    ranked
        .into_iter()
        .filter_map(|(id, count)| {
            paths.get(id).map(|path| CampaignPathCount {
                id: *id,
                path: path.clone(),
                count: *count,
            })
        })
        .collect()
}

fn preview_rules(
    conn: &ProjectConnection,
    rules: &[CampaignRuleInput],
    query: &CampaignsQuery,
) -> Result<CampaignPreviewResponse, AuthError> {
    let compiled: Vec<CampaignRuleMatch> = rules
        .iter()
        .map(|rule| CampaignRuleMatch {
            campaign_id: 0,
            param_key: rule.param_key.clone(),
            param_value: rule.param_value.clone(),
        })
        .collect();
    let country_filter = query.segments.country_in_clause("l.country", 3);
    let session_filter = query
        .segments
        .session_filter_clause("l.visitor_timestamp_id", Some(SegmentDimension::Source));
    let sql = format!(
        "SELECT l.visitor_timestamp_id, l.campaign IS NOT NULL, r.referrer
         FROM landings l
         LEFT JOIN referrers r ON r.id = l.referrer
         WHERE l.visitor_timestamp_id >= ?1
           AND l.visitor_timestamp_id < ?2{country_filter}{session_filter}"
    );
    let start_ns = query.start * NANOS_PER_SECOND;
    let end_ns = query.end * NANOS_PER_SECOND;
    let mut sql_params: Vec<&dyn ToSql> = vec![&start_ns, &end_ns];
    query.segments.append_country_params(&mut sql_params);
    let landings = conn
        .with_query(&sql, params_from_iter(sql_params), |mut rows| {
            let mut landings = Vec::new();
            while let Some(row) = rows.next()? {
                let classified: i64 = row.get(1)?;
                landings.push(PreviewLanding {
                    visitor_timestamp_id: row.get(0)?,
                    classified: classified != 0,
                    referrer: row.get(2)?,
                });
            }
            Ok(landings)
        })
        .map_err(map_row_error)?;

    let param_keys: Vec<&str> = compiled
        .iter()
        .map(|rule| rule.param_key.as_str())
        .collect();
    let metadata = if param_keys.is_empty() {
        FxHashMap::default()
    } else {
        load_preview_metadata(conn, query, &param_keys)?
    };

    let mut response = CampaignPreviewResponse {
        unclassified: 0,
        direct: 0,
        referrer: 0,
        already_classified: 0,
    };
    for landing in &landings {
        let Some(params) = metadata.get(&landing.visitor_timestamp_id) else {
            continue;
        };
        if !compiled.iter().any(|rule| rule_matches(rule, params)) {
            continue;
        }
        if landing.classified {
            response.already_classified += 1;
        } else if landing.referrer.is_some() {
            response.unclassified += 1;
            response.referrer += 1;
        } else {
            response.unclassified += 1;
            response.direct += 1;
        }
    }
    Ok(response)
}

struct PreviewLanding {
    visitor_timestamp_id: i64,
    classified: bool,
    referrer: Option<String>,
}

fn load_preview_metadata(
    conn: &ProjectConnection,
    query: &CampaignsQuery,
    keys: &[&str],
) -> Result<FxHashMap<i64, serde_json::Map<String, Value>>, AuthError> {
    let country_filter = query.segments.country_in_clause("l.country", 3);
    let session_filter = query
        .segments
        .session_filter_clause("l.visitor_timestamp_id", Some(SegmentDimension::Source));
    let key_list = keys
        .iter()
        .map(|key| format!("'{}'", key.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT m.visitor_timestamp_id, m.key, m.value
         FROM metadata m
         JOIN landings l ON l.visitor_timestamp_id = m.visitor_timestamp_id
         WHERE l.visitor_timestamp_id >= ?1
           AND l.visitor_timestamp_id < ?2
           AND m.key IN ({key_list}){country_filter}{session_filter}"
    );
    let start_ns = query.start * NANOS_PER_SECOND;
    let end_ns = query.end * NANOS_PER_SECOND;
    let mut sql_params: Vec<&dyn ToSql> = vec![&start_ns, &end_ns];
    query.segments.append_country_params(&mut sql_params);
    conn.with_query(&sql, params_from_iter(sql_params), |mut rows| {
        let mut metadata: FxHashMap<i64, serde_json::Map<String, Value>> = FxHashMap::default();
        while let Some(row) = rows.next()? {
            let visitor_timestamp_id: i64 = row.get(0)?;
            let key: String = row.get(1)?;
            let value: String = row.get(2)?;
            metadata
                .entry(visitor_timestamp_id)
                .or_default()
                .insert(key, Value::String(value));
        }
        Ok(metadata)
    })
    .map_err(map_row_error)
}

fn save_campaign_on(
    conn: &ProjectConnection,
    request: &SaveCampaignRequest,
) -> Result<SaveCampaignResponse, AuthError> {
    in_transaction(conn, |conn| {
        let id = if let Some(id) = request.id {
            let exists = conn
                .query_row(
                    "SELECT 1 FROM campaigns WHERE id = ?1 AND deleted_at IS NULL",
                    params![id],
                    |_| Ok(()),
                )
                .optional()
                .map_err(map_row_error)?;
            if exists.is_none() {
                return Err(AuthError::InvalidRequest("campaign not found"));
            }
            conn.execute(
                "UPDATE campaigns SET display_name = ?1 WHERE id = ?2",
                params![request.name, id],
            )
            .map_err(map_row_error)?;
            if request.rules.is_empty() {
                return Ok(SaveCampaignResponse { id, assigned: 0 });
            }
            id
        } else {
            let campaign_key = request.name.to_lowercase();
            if let Some((id, _)) = find_named_campaign(conn, &campaign_key)? {
                conn.execute(
                    "UPDATE campaigns SET display_name = ?1, deleted_at = NULL WHERE id = ?2",
                    params![request.name, id],
                )
                .map_err(map_row_error)?;
                let mut merged = load_rule_inputs(conn, id)?;
                merged.extend(request.rules.clone());
                let merged = dedup_rules(merged).map_err(AuthError::InvalidRequest)?;
                if merged.len() > MAX_RULES {
                    return Err(AuthError::InvalidRequest("too many rules"));
                }
                replace_rules(conn, id, &merged)?;
                let assigned = backfill_campaign(conn, id, &merged)?;
                refresh_first_seen(conn, id)?;
                return Ok(SaveCampaignResponse { id, assigned });
            } else {
                let first_seen = i64::try_from(nanoseconds_since_epoch()).unwrap_or(0);
                conn.execute(
                    "INSERT INTO campaigns (source, medium, campaign, display_name, first_seen_at)
                     VALUES ('', '', ?1, ?2, ?3)",
                    params![campaign_key, request.name, first_seen],
                )
                .map_err(map_row_error)?;
                conn.query_row(
                    "SELECT id FROM campaigns
                     WHERE source = '' AND medium = '' AND campaign = ?1 AND network IS NULL",
                    params![campaign_key],
                    |row| row.get(0),
                )
                .map_err(map_row_error)?
            }
        };
        replace_rules(conn, id, &request.rules)?;
        let assigned = backfill_campaign(conn, id, &request.rules)?;
        refresh_first_seen(conn, id)?;
        Ok(SaveCampaignResponse { id, assigned })
    })
}

fn find_named_campaign(
    conn: &ProjectConnection,
    campaign_key: &str,
) -> Result<Option<(i64, bool)>, AuthError> {
    conn.query_row(
        "SELECT id, deleted_at IS NOT NULL
         FROM campaigns
         WHERE source = '' AND medium = '' AND campaign = ?1 AND network IS NULL",
        params![campaign_key],
        |row| {
            let deleted: i64 = row.get(1)?;
            Ok((row.get(0)?, deleted != 0))
        },
    )
    .optional()
    .map_err(map_row_error)
}

fn load_rule_inputs(
    conn: &ProjectConnection,
    campaign_id: i64,
) -> Result<Vec<CampaignRuleInput>, AuthError> {
    conn.with_query(
        "SELECT param_key, param_value
         FROM campaign_rules
         WHERE campaign_id = ?1
         ORDER BY id",
        params![campaign_id],
        |mut rows| {
            let mut rules = Vec::new();
            while let Some(row) = rows.next()? {
                rules.push(CampaignRuleInput {
                    param_key: row.get(0)?,
                    param_value: row.get(1)?,
                });
            }
            Ok(rules)
        },
    )
    .map_err(map_row_error)
}

fn replace_rules(
    conn: &ProjectConnection,
    campaign_id: i64,
    rules: &[CampaignRuleInput],
) -> Result<(), AuthError> {
    conn.execute(
        "DELETE FROM campaign_rules WHERE campaign_id = ?1",
        params![campaign_id],
    )
    .map_err(map_row_error)?;
    for rule in rules {
        conn.execute(
            "INSERT INTO campaign_rules (campaign_id, param_key, param_value)
             VALUES (?1, ?2, ?3)",
            params![campaign_id, rule.param_key, rule.param_value],
        )
        .map_err(map_row_error)?;
    }
    Ok(())
}

fn backfill_campaign(
    conn: &ProjectConnection,
    campaign_id: i64,
    rules: &[CampaignRuleInput],
) -> Result<i64, AuthError> {
    if rules.is_empty() {
        return Ok(0);
    }
    let clauses: Vec<String> = (0..rules.len())
        .map(|index| {
            let key = 2 + index * 2;
            let value = key + 1;
            format!("(m.key = ?{key} COLLATE NOCASE AND m.value = ?{value} COLLATE NOCASE)")
        })
        .collect();
    let sql = format!(
        "UPDATE landings
         SET campaign = ?1
         WHERE campaign IS NULL
           AND visitor_timestamp_id IN (
               SELECT m.visitor_timestamp_id
               FROM metadata m
               WHERE {}
           )",
        clauses.join(" OR ")
    );
    let mut sql_params: Vec<&dyn ToSql> = Vec::with_capacity(1 + rules.len() * 2);
    sql_params.push(&campaign_id);
    for rule in rules {
        sql_params.push(&rule.param_key);
        sql_params.push(&rule.param_value);
    }
    let assigned = conn
        .execute(&sql, params_from_iter(sql_params))
        .map_err(map_row_error)?;
    Ok(assigned as i64)
}

fn refresh_first_seen(conn: &ProjectConnection, campaign_id: i64) -> Result<(), AuthError> {
    conn.execute(
        "UPDATE campaigns
         SET first_seen_at = (
             SELECT MIN(visitor_timestamp_id) FROM landings WHERE campaign = ?1
         )
         WHERE id = ?1
           AND EXISTS (SELECT 1 FROM landings WHERE campaign = ?1)",
        params![campaign_id],
    )
    .map_err(map_row_error)?;
    Ok(())
}

pub(crate) fn in_transaction<T>(
    conn: &ProjectConnection,
    body: impl FnOnce(&ProjectConnection) -> Result<T, AuthError>,
) -> Result<T, AuthError> {
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(map_row_error)?;
    match body(conn) {
        Ok(value) => {
            conn.execute_batch("COMMIT").map_err(map_row_error)?;
            Ok(value)
        }
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

fn normalize_display_name(name: &str) -> Result<String, &'static str> {
    let name = name.trim();
    if name.is_empty() || name.len() > MAX_NAME_LEN {
        return Err("name is required");
    }
    if name.to_lowercase().is_empty() {
        return Err("name is required");
    }
    Ok(name.to_string())
}

fn normalize_rules(rules: Vec<CampaignRuleInput>) -> Result<Vec<CampaignRuleInput>, &'static str> {
    if rules.is_empty() {
        return Err("at least one rule is required");
    }
    if rules.len() > MAX_RULES {
        return Err("too many rules");
    }
    let mut normalized = Vec::with_capacity(rules.len());
    for rule in rules {
        normalized.push(CampaignRuleInput {
            param_key: normalize_param_key(&rule.param_key)?,
            param_value: normalize_param_value(&rule.param_value)?,
        });
    }
    dedup_rules(normalized)
}

fn dedup_rules(rules: Vec<CampaignRuleInput>) -> Result<Vec<CampaignRuleInput>, &'static str> {
    let mut unique = Vec::with_capacity(rules.len());
    for rule in rules {
        let duplicate = unique.iter().any(|existing: &CampaignRuleInput| {
            existing.param_key == rule.param_key
                && existing.param_value.eq_ignore_ascii_case(&rule.param_value)
        });
        if !duplicate {
            unique.push(rule);
        }
    }
    if unique.is_empty() {
        return Err("at least one rule is required");
    }
    Ok(unique)
}

fn normalize_param_key(value: &str) -> Result<String, &'static str> {
    let trimmed = value.trim().to_ascii_lowercase();
    let key = trimmed
        .strip_prefix("utm_")
        .or_else(|| trimmed.strip_prefix("rr_"))
        .unwrap_or(trimmed.as_str());
    if key.is_empty()
        || key.len() > MAX_PARAM_KEY_LEN
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err("param_key is invalid");
    }
    Ok(key.to_string())
}

fn normalize_param_value(value: &str) -> Result<String, &'static str> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_PARAM_VALUE_LEN {
        return Err("param_value is invalid");
    }
    Ok(value.to_string())
}

#[cfg(test)]
mod tests {
    use crate::test_utils::open_migrated_db;

    use rusqlite::params;
    use serde_json::json;

    use super::*;

    use crate::test_utils::{PaymentFixture, insert_payment};

    /// The project comes from the path, and the dashboard sends no `project_id`.
    #[test]
    fn campaign_payloads_deserialize_without_a_project_id() {
        let rules = r#"[{"param_key":"ref","param_value":"producthunt"}]"#;
        serde_json::from_str::<SaveCampaignRequest>(&format!(
            r#"{{"name":"producthunt","rules":{rules}}}"#
        ))
        .expect("save payload");
        serde_json::from_str::<CampaignPreviewRequest>(&format!(r#"{{"rules":{rules}}}"#))
            .expect("preview payload");
    }

    const START: i64 = 1_700_000_000;
    const END: i64 = START + 10 * 86_400;

    fn timestamp(seed: i64) -> i64 {
        (START + 3_600) * NANOS_PER_SECOND + seed
    }

    fn insert_path(conn: &ProjectConnection, path: &str) -> i64 {
        conn.execute(
            "INSERT INTO paths (path, hostname) VALUES (?1, 'acme.test')",
            params![path],
        )
        .expect("path");
        conn.query_row(
            "SELECT id FROM paths WHERE path = ?1",
            params![path],
            |row| row.get(0),
        )
        .expect("path id")
    }

    fn insert_referrer(conn: &ProjectConnection, referrer: &str) -> i64 {
        conn.execute(
            "INSERT INTO referrers (referrer) VALUES (?1)",
            params![referrer],
        )
        .expect("referrer");
        conn.query_row(
            "SELECT id FROM referrers WHERE referrer = ?1",
            params![referrer],
            |row| row.get(0),
        )
        .expect("referrer id")
    }

    fn insert_visit(
        conn: &ProjectConnection,
        seed: i64,
        path_id: i64,
        referrer_id: Option<i64>,
        campaign_id: Option<i64>,
        country: Option<&str>,
        metadata: &[(&str, &str)],
    ) -> i64 {
        let timestamp_id = timestamp(seed);
        conn.execute(
            "INSERT INTO pageviews (
                timestamp_id, visitor_timestamp_id, path_id
             ) VALUES (?1, ?1, ?2)",
            params![timestamp_id, path_id],
        )
        .expect("pageview");
        conn.execute(
            "INSERT INTO landings (
                visitor_timestamp_id, referrer, campaign, is_google, country,
                tracker_version, device
             ) VALUES (?1, ?2, ?3, 0, ?4, 1.0, 'desktop')",
            params![timestamp_id, referrer_id, campaign_id, country],
        )
        .expect("landing");
        for (key, value) in metadata {
            conn.execute(
                "INSERT INTO metadata (key, value, visitor_timestamp_id, page_timestamp_id)
                 VALUES (?1, ?2, ?3, ?3)",
                params![key, value, timestamp_id],
            )
            .expect("metadata");
        }
        timestamp_id
    }

    fn query() -> CampaignsQuery {
        CampaignsQuery {
            start: START,
            end: END,
            segments: SegmentFilters::default(),
        }
    }

    fn save(
        conn: &ProjectConnection,
        name: &str,
        rules: Vec<CampaignRuleInput>,
    ) -> SaveCampaignResponse {
        save_campaign_on(
            conn,
            &SaveCampaignRequest {
                id: None,
                name: name.to_string(),
                rules,
            },
        )
        .expect("save campaign")
    }

    #[test]
    fn rejects_a_query_without_a_range() {
        assert!(parse_campaigns_query("/v2/read/projects/1/campaigns").is_err());
    }

    fn rule(key: &str, value: &str) -> CampaignRuleInput {
        CampaignRuleInput {
            param_key: key.to_string(),
            param_value: value.to_string(),
        }
    }

    #[test]
    fn strips_utm_prefix_and_trims_values() {
        let request = normalize_save_request(SaveCampaignRequest {
            id: None,
            name: "  BetaList  ".to_string(),
            rules: vec![rule("utm_source", " BetaList "), rule("source", "betalist")],
        })
        .expect("normalize");
        assert_eq!(request.name, "BetaList");
        assert_eq!(request.rules.len(), 1);
        assert_eq!(request.rules[0].param_key, "source");
        assert_eq!(request.rules[0].param_value, "BetaList");
    }

    #[test]
    fn rejects_a_rule_without_a_value() {
        let error = normalize_save_request(SaveCampaignRequest {
            id: None,
            name: "BetaList".to_string(),
            rules: vec![rule("source", " ")],
        })
        .expect_err("blank value");
        assert_eq!(error, "param_value is invalid");
    }

    #[test]
    fn assigns_unclassified_landings_and_leaves_existing_campaigns() {
        let conn = open_migrated_db();
        let path_id = insert_path(&conn, "/");
        let betalist = insert_referrer(&conn, "betalist.com");
        let existing = conn
            .query_row(
                "INSERT INTO campaigns (source, medium, campaign, first_seen_at)
                 VALUES ('newsletter', 'email', 'launch', 1)
                 RETURNING id",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("existing campaign");
        let source_visit = insert_visit(
            &conn,
            1,
            path_id,
            None,
            None,
            Some("US"),
            &[("source", "betalist"), ("content", "header")],
        );
        let ref_visit = insert_visit(
            &conn,
            2,
            path_id,
            Some(betalist),
            None,
            Some("DE"),
            &[("ref", "betalist")],
        );
        let referrer_only = insert_visit(&conn, 3, path_id, Some(betalist), None, None, &[]);
        let already = insert_visit(
            &conn,
            4,
            path_id,
            None,
            Some(existing),
            None,
            &[("source", "betalist")],
        );
        let mut payment = PaymentFixture::new("pay_betalist", 12_500, START + 4_000);
        payment.visitor_timestamp_id = Some(source_visit);
        insert_payment(&conn, payment);

        let rules = vec![rule("source", "betalist"), rule("ref", "BetaList")];
        let preview = preview_rules(&conn, &rules, &query()).expect("preview");
        assert_eq!(preview.direct, 1);
        assert_eq!(preview.referrer, 1);
        assert_eq!(preview.unclassified, 2);
        assert_eq!(preview.already_classified, 1);

        let saved = save(&conn, "BetaList", rules);
        assert_eq!(saved.assigned, 2);

        let response = build_campaigns_response_at(&conn, &query(), &ProjectTimezone::utc(), END)
            .expect("list");
        let betalist_row = response
            .campaigns
            .iter()
            .find(|row| row.display_name.as_deref() == Some("BetaList"))
            .expect("betalist campaign");
        assert_eq!(betalist_row.visitors, 2);
        assert_eq!(betalist_row.revenue, 12_500);
        let interval = betalist_row.conversion_ci.expect("interval");
        assert!(interval.lower < 0.5 && interval.upper > 0.5);
        assert_eq!(betalist_row.spend, 0);
        assert_eq!(betalist_row.roi, None);
        assert_eq!(betalist_row.contents[0].label, "header");
        assert!(
            betalist_row
                .referrers
                .iter()
                .any(|entry| entry.label == "betalist.com")
        );
        assert!(response.suggestions.is_empty());

        let still_existing: i64 = conn
            .query_row(
                "SELECT campaign FROM landings WHERE visitor_timestamp_id = ?1",
                params![already],
                |row| row.get(0),
            )
            .expect("existing landing");
        assert_eq!(still_existing, existing);
        let _ = (source_visit, ref_visit, referrer_only);
    }

    #[test]
    fn suggests_unclassified_source_and_ref_outside_other_channels() {
        let conn = open_migrated_db();
        let path_id = insert_path(&conn, "/pricing");
        let referrer = insert_referrer(&conn, "news.ycombinator.com");
        let chatgpt = insert_referrer(&conn, "chatgpt.com");
        insert_visit(
            &conn,
            1,
            path_id,
            None,
            None,
            None,
            &[("source", "hn"), ("ref", "launch")],
        );
        insert_visit(&conn, 2, path_id, Some(referrer), None, None, &[]);
        let ai_visit = insert_visit(
            &conn,
            3,
            path_id,
            Some(chatgpt),
            None,
            None,
            &[("source", "chatgpt")],
        );
        conn.execute(
            "UPDATE landings SET referrer_type = 'ai' WHERE visitor_timestamp_id = ?1",
            params![ai_visit],
        )
        .expect("ai landing");
        let response = build_campaigns_response_at(&conn, &query(), &ProjectTimezone::utc(), END)
            .expect("list");
        assert!(
            !response
                .suggestions
                .iter()
                .any(|item| item.value == "chatgpt")
        );
        assert!(response.campaigns.is_empty());
        assert!(
            response
                .suggestions
                .iter()
                .any(|item| { item.kind == "source" && item.value == "hn" && item.visitors == 1 })
        );
        assert!(
            response
                .suggestions
                .iter()
                .any(|item| { item.kind == "ref" && item.value == "launch" })
        );
        assert!(
            response
                .suggestions
                .iter()
                .all(|item| item.kind == "source" || item.kind == "ref")
        );
    }

    #[test]
    fn hiding_a_campaign_keeps_its_visits() {
        let conn = open_migrated_db();
        let path_id = insert_path(&conn, "/");
        insert_visit(
            &conn,
            1,
            path_id,
            None,
            None,
            None,
            &[("source", "betalist")],
        );
        let saved = save(&conn, "BetaList", vec![rule("source", "betalist")]);
        conn.execute(
            "UPDATE campaigns SET deleted_at = unixepoch() WHERE id = ?1",
            params![saved.id],
        )
        .expect("hide");
        let response = build_campaigns_response_at(&conn, &query(), &ProjectTimezone::utc(), END)
            .expect("list");
        assert!(response.campaigns.is_empty());
        let campaign: Option<i64> = conn
            .query_row("SELECT campaign FROM landings", [], |row| row.get(0))
            .expect("landing");
        assert_eq!(campaign, Some(saved.id));
    }

    #[test]
    fn matching_uses_the_first_matching_rule() {
        let rules = [
            CampaignRuleMatch {
                campaign_id: 7,
                param_key: "source".to_string(),
                param_value: "betalist".to_string(),
            },
            CampaignRuleMatch {
                campaign_id: 9,
                param_key: "ref".to_string(),
                param_value: "betalist".to_string(),
            },
        ];
        let source = json!({"source": "BetaList"});
        let referral = json!({"ref": "betalist"});
        let other = json!({"source": "hn"});
        assert_eq!(matching_campaign_id(&rules, source.as_object()), Some(7));
        assert_eq!(matching_campaign_id(&rules, referral.as_object()), Some(9));
        assert_eq!(matching_campaign_id(&rules, other.as_object()), None);
        assert_eq!(matching_campaign_id(&rules, None), None);
    }
}
