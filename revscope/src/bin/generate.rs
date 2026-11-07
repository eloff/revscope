use rand::{
    Rng, RngCore, SeedableRng,
    distributions::{Distribution, weighted::WeightedIndex},
    prelude::SliceRandom,
    rngs::StdRng,
};
use revscope::util::NANOS_PER_SECOND;
use revscope::{
    ReferrerType, SourceGroup, classify_referrer_type, click_id_network_from_map,
    db::{CAMPAIGNS_INSERT_ON_CONFLICT, ProjectConnection as Connection},
    major_cities_for_country, referrer_is_google, source_group_for_landing, upsert_crawler_path,
    upsert_useragent,
    util::nanoseconds_since_epoch,
};
use rusqlite::OptionalExtension;
use rusqlite::params;
use serde_json::Value;
use serde_json::json;
use std::{
    collections::HashSet,
    error::Error,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

const DAY_SECONDS: i64 = 86_400;
const HOUR_SECONDS: i64 = 3_600;
const MONTH_SECONDS: i64 = 30 * DAY_SECONDS;
const YEAR_SECONDS: i64 = 12 * MONTH_SECONDS;
/// Share of new subscriptions billed annually (picked once per dataset).
const ANNUAL_SUBSCRIPTION_SHARE_MIN: f64 = 0.30;
const ANNUAL_SUBSCRIPTION_SHARE_MAX: f64 = 0.45;
/// Month-over-month subscription revenue growth (never decreases).
const MIN_MONTH_REVENUE_GROWTH: f64 = 1.01;
const MAX_MONTH_REVENUE_GROWTH: f64 = 1.25;
/// Log visitor-loop progress every N sessions (each session is several INSERTs).
const VISITOR_PROGRESS_INTERVAL: usize = 5_000;
/// Baseline visitor sessions per conversion; month jitter is ±30%.
const VISITORS_PER_CONVERSION: f64 = 2000.0;
const VISITOR_MONTH_JITTER: f64 = 0.30;
const DAYS_PER_MONTH: usize = 30;
/// Shared daily revenue shape (quietest vs busiest day).
const DAY_REVENUE_WEIGHT_MIN: f64 = 1.0;
const DAY_REVENUE_WEIGHT_MAX: f64 = 2.5;
/// Low non-converting traffic baseline so zero-conversion days still get realistic visits.
const VISITOR_BASELINE_CONVERSION_SHARE: f64 = 0.15;
/// Per-day noise when mapping planned conversions → visitors (keeps correlation, adds spread).
const VISITOR_DAY_NOISE_MIN: f64 = 0.80;
const VISITOR_DAY_NOISE_MAX: f64 = 1.20;
const MIN_REFUNDS_PER_MONTH: usize = 2;
const MIN_CHARGEBACKS_PER_MONTH: usize = 1;
/// Target share of visitor sessions that bounce (single pageview, under 10s).
const BOUNCE_SESSION_PROB: f64 = 0.14;
/// Minimum summed pageview duration for converting sessions (extended on payment).
const MIN_CONVERSION_SESSION_DURATION_SECONDS: i64 = 300;
/// Minimum wall-clock time from first visit to payment.
const MIN_TIME_TO_PURCHASE_SECONDS: i64 = MIN_CONVERSION_SESSION_DURATION_SECONDS;
/// Share of converting customers who return to the product after purchase.
const POST_CONVERSION_RETURN_PROB: f64 = 0.92;
/// Path indices for post-purchase sessions (dashboard, docs, occasional marketing).
const POST_CONVERSION_PATH_INDICES: [usize; 6] = [11, 6, 7, 8, 1, 2];
const POST_CONVERSION_PATH_WEIGHTS: [u32; 6] = [450, 150, 200, 150, 30, 20];
/// Event name indices into `upsert_event_names` for post-purchase product usage.
const POST_CONVERSION_EVENT_INDICES: [usize; 6] = [8, 9, 10, 11, 12, 13];
/// Share of churned subscription customers who open the cancel flow before canceling.
const CANCEL_FLOW_BEFORE_CHURN_SHARE: f64 = 0.70;
/// Minimum time after a payment before a customer can open the cancel flow.
const MIN_CANCEL_FLOW_AFTER_PAYMENT_SECONDS: i64 = 14 * DAY_SECONDS;
const CANCEL_FLOW_EVENT_INDEX: usize = 14;
/// Outbound link clicks (hostname in event metadata `to`).
const OUTBOUND_CLICK_EVENT_INDEX: usize = 15;
/// Share of eligible pageviews that emit a revscope-outbound-click event.
const OUTBOUND_CLICK_PROB: f64 = 0.11;
/// Occasional product event on non-converting visitor sessions (not a funnel step).
const VISITOR_SESSION_EVENT_INDEX: usize = 8;
/// Home landing conversion funnel events, in step order after the `/` pageview goal.
const HOME_FUNNEL_EVENT_INDICES: [usize; 8] = [0, 1, 2, 3, 4, 5, 6, 7];
const HOME_FUNNEL_EVENT_DESCRIPTIONS: [&str; 8] = [
    "Viewed problem section",
    "Viewed solution section",
    "Viewed pricing section",
    "Clicked primary call-to-action",
    "Started signup flow",
    "Completed account signup",
    "Selected a subscription plan",
    "Started checkout",
];
/// Per-step continuation probability for non-converting sessions (after landing and each event).
const HOME_FUNNEL_STEP_CONTINUE: [f64; 8] = [0.92, 0.82, 0.74, 0.28, 0.48, 0.74, 0.65, 0.38];
/// Share of non-bounce sessions with a `/` pageview that enter the home conversion funnel.
const HOME_FUNNEL_ATTEMPT_PROB: f64 = 0.32;
/// Inclusive event-step index for "Completed account signup".
const HOME_FUNNEL_COMPLETED_SIGNUP_STEP: usize = 5;
/// Inclusive event-step index for "Started checkout".
const HOME_FUNNEL_STARTED_CHECKOUT_STEP: usize = 7;
const HOME_LANDING_GOAL_DESCRIPTION: &str = "Visited marketing homepage";
const HOME_CONVERSION_FUNNEL_NAME: &str = "Home Landing Conversions";
const SEARCH_QUERY_COUNT: usize = 1000;
/// Total daily impressions at the busiest end-of-range day, split across queries via Pareto.
const SEARCH_PEAK_IMPRESSIONS_TOTAL: i32 = 880;
/// Pareto shape for query-level impression share (lower = more head-heavy).
const SEARCH_PARETO_ALPHA_MIN: f64 = 0.90;
const SEARCH_PARETO_ALPHA_MAX: f64 = 1.10;
/// Skew daily impression volume toward the end of the range (higher = steeper ramp).
const SEARCH_DATE_POWER_MIN: f64 = 1.6;
const SEARCH_DATE_POWER_MAX: f64 = 3.4;
/// Share of landings per channel (six buckets).
/// Kept close to conversion targets so preferred-group lookups usually find a
/// same-day visitor. Search is slightly above the Search conversion target so
/// that after `SEARCH_GOOGLE_ADS_SHARE` siphons some into Campaigns, organic
/// search lands near ~18–20% of visitors.
const LANDING_CHANNEL_SEARCH: f64 = 0.20;
/// Classified as `referrer_type = ai`; kept near the AI conversion target.
const LANDING_CHANNEL_AI: f64 = 0.16;
const LANDING_CHANNEL_SOCIAL: f64 = 0.18;
const LANDING_CHANNEL_CAMPAIGN: f64 = 0.18;
const LANDING_CHANNEL_REFERRER: f64 = 0.18;
const LANDING_CHANNEL_DIRECT: f64 = 0.10;
/// Target share of *new-conversion revenue* by revenue-source group.
/// Assigned greedily by payment amount each month (not by conversion count) so
/// annual invoices cannot pile into one channel. Direct is capped at 10%; every
/// other channel targets 16–20% (within 10–25%).
const CONVERSION_GROUP_SEARCH: f64 = 0.18;
const CONVERSION_GROUP_AI: f64 = 0.18;
const CONVERSION_GROUP_SOCIAL: f64 = 0.18;
const CONVERSION_GROUP_CAMPAIGNS: f64 = 0.20;
const CONVERSION_GROUP_REFERRERS: f64 = 0.18;
/// Slightly under 10% so discrete annual invoices rarely push Direct over the cap.
const CONVERSION_GROUP_DIRECT: f64 = 0.08;
const SOURCE_GROUP_COUNT: usize = 6;
const CONVERSION_GROUP_WEIGHTS: [f64; SOURCE_GROUP_COUNT] = [
    CONVERSION_GROUP_SEARCH,
    CONVERSION_GROUP_AI,
    CONVERSION_GROUP_SOCIAL,
    CONVERSION_GROUP_CAMPAIGNS,
    CONVERSION_GROUP_REFERRERS,
    CONVERSION_GROUP_DIRECT,
];

#[derive(Clone)]
struct ActiveSubscription {
    id: i64,
    amount: i64,
    interval_unit: &'static str,
    current_period_end: i64,
    provider_customer_id: String,
    customer_email: String,
    visitor_timestamp_id: i64,
    last_paid_at: i64,
}

struct PendingCancellation {
    subscription_id: i64,
    canceled_at: i64,
    /// Recurring price per billing interval, in thousandths.
    amount: i64,
    monthly_amount: i64,
    interval_unit: &'static str,
    visitor_timestamp_id: i64,
    last_paid_at: i64,
    emit_cancel_flow: bool,
}

/// One appended state of a subscription, mirroring what a provider sync writes
/// to `subscription_history`.
struct SubscriptionVersion {
    effective_from: i64,
    change_kind: &'static str,
    status: &'static str,
    plan_name: &'static str,
    /// Recurring price per billing interval, in thousandths.
    plan_amount: i64,
    mrr_amount: i64,
    interval_unit: &'static str,
    canceled_at: Option<i64>,
    ended_at: Option<i64>,
}

struct PendingRenewal {
    subscription: ActiveSubscription,
    paid_day: usize,
    paid_at: i64,
    period_end: i64,
}

struct PendingNewSubscription {
    amount: i64,
    monthly_amount: i64,
    interval_unit: &'static str,
    paid_day: usize,
    paid_at: i64,
    period_end: i64,
}

struct PendingSinglePayment {
    amount: i64,
    paid_day: usize,
    paid_at: i64,
}

struct PaymentIdState {
    payment_seq: u64,
    invoice_seq: u64,
    order_seq: u64,
}

#[derive(Clone, Copy)]
enum NonConversionSessionShape {
    /// Single pageview under 10 seconds (overview bounce definition).
    Bounce,
    /// Brief visit, usually one page.
    Short,
    /// Multi-page visit with moderate engagement.
    Medium,
}

fn pick_non_conversion_session_shape(rng: &mut StdRng) -> NonConversionSessionShape {
    if rng.gen_bool(BOUNCE_SESSION_PROB) {
        return NonConversionSessionShape::Bounce;
    }
    if rng.gen_bool(0.72) {
        NonConversionSessionShape::Short
    } else {
        NonConversionSessionShape::Medium
    }
}

fn run(
    project_id: &str,
    data_dir: &str,
    empty: bool,
    overwrite: bool,
) -> Result<(), Box<dyn Error>> {
    println!("generate: starting for project {project_id} (data_dir={data_dir})");
    if overwrite {
        remove_destination_db_files(data_dir, project_id)?;
    }
    let server_context = Arc::new(revscope::server_context::ServerContext::new(data_dir));
    let project_key = revscope::db::parse_project_id(project_id)?;
    // Local seed tool: identity is not loaded from Postgres.
    revscope::db::insert_project(
        server_context.as_ref(),
        project_key,
        format!("generate_{project_id}"),
        1,
    );
    println!("generate: creating project database…");
    revscope::db::create_project_database(server_context.as_ref(), project_id)?;
    if empty {
        println!("generate: empty project database ready at {data_dir}/{project_id}.db");
        return Ok(());
    }

    let mut rng = StdRng::seed_from_u64(2026051801);
    let annual_subscription_share =
        rng.gen_range(ANNUAL_SUBSCRIPTION_SHARE_MIN..=ANNUAL_SUBSCRIPTION_SHARE_MAX);
    println!(
        "generate: subscription mix — {:.0}% annual / {:.0}% monthly",
        annual_subscription_share * 100.0,
        (1.0 - annual_subscription_share) * 100.0,
    );
    let conn = revscope::db::open_project_database_write(server_context.as_ref(), project_id)?;
    println!("generate: database open, beginning transaction");
    conn.execute_batch(
        "PRAGMA synchronous = NORMAL;
         BEGIN IMMEDIATE;",
    )?;

    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;
    let start_ts = now - (6 * MONTH_SECONDS);

    let mut monthly_targets = Vec::with_capacity(6);
    monthly_targets.push(7_746_200_i64);
    for _ in 1..6 {
        let prev = *monthly_targets.last().expect("monthly targets non-empty") as f64;
        let factor = rng.gen_range(MIN_MONTH_REVENUE_GROWTH..=MAX_MONTH_REVENUE_GROWTH);
        monthly_targets.push((prev * factor).round() as i64);
    }
    let penultimate = monthly_targets[4];
    println!(
        "generate: monthly subscription targets — month 5 ${:.2}, month 6 ${:.2} ({:.1}% vs prior)",
        penultimate as f64 / 1000.0,
        monthly_targets[5] as f64 / 1000.0,
        (monthly_targets[5] as f64 / penultimate as f64 - 1.0) * 100.0,
    );

    println!("generate: upserting reference data (paths, referrers, events)…");
    let path_ids = upsert_paths(&conn)?;
    let landing_path_pickers = LandingPathPickers::new(&mut rng)?;
    let post_conversion_paths = WeightedIndex::new(POST_CONVERSION_PATH_WEIGHTS)?;
    let locale_pickers = LocalePickers::new()?;
    let os_pickers = OsPickers::new()?;
    let browser_pickers = BrowserPickers::new()?;
    let referrer_ids = upsert_referrers(&conn)?;
    let referrer_pickers = ReferrerPickers::new()?;
    let event_name_ids = upsert_event_names(&conn)?;
    println!("generate: reference data ready, generating 6 months of traffic");

    let mut active_subscriptions: Vec<ActiveSubscription> = Vec::new();
    let mut payment_ids = PaymentIdState {
        payment_seq: 1,
        invoice_seq: 1,
        order_seq: 1,
    };
    let mut created_subscription_seq = 1_u64;

    for (month_idx, target_subscription_revenue) in monthly_targets.iter().enumerate() {
        let month_start = start_ts + (month_idx as i64 * MONTH_SECONDS);
        let month_end = month_start + MONTH_SECONDS - 1;

        let day_shape = generate_day_revenue_weights(&mut rng);
        let mut day_subscription_budget =
            allocate_i64_by_weights(*target_subscription_revenue, &day_shape);
        let single_payment_count = rng.gen_range(10..=32);
        let single_amounts: Vec<i64> = (0..single_payment_count)
            .map(|_| rng.gen_range(7_500..=180_000))
            .collect();
        let single_total: i64 = single_amounts.iter().sum();
        let mut day_single_budget = allocate_i64_by_weights(single_total, &day_shape);
        let mut day_conversion_counts = vec![0_usize; DAYS_PER_MONTH];

        let churn_rate = rng.gen_range(0.07_f64..0.18_f64);
        let mut pending_cancellations = Vec::new();
        let mut pending_renewals = Vec::new();
        let mut carried_subscriptions = Vec::with_capacity(active_subscriptions.len());
        let mut month_subscription_revenue = 0_i64;

        for existing in active_subscriptions.drain(..) {
            if rng.gen_bool(churn_rate) {
                pending_cancellations.push(PendingCancellation {
                    subscription_id: existing.id,
                    canceled_at: rng.gen_range(month_start..=month_end),
                    amount: existing.amount,
                    monthly_amount: subscription_monthly_amount(
                        existing.amount,
                        existing.interval_unit,
                    ),
                    interval_unit: existing.interval_unit,
                    visitor_timestamp_id: existing.visitor_timestamp_id,
                    last_paid_at: existing.last_paid_at,
                    emit_cancel_flow: rng.gen_bool(CANCEL_FLOW_BEFORE_CHURN_SHARE),
                });
                continue;
            }

            let renews_this_month =
                existing.interval_unit == "month" || existing.current_period_end <= month_end;
            if renews_this_month {
                let paid_day =
                    schedule_payment_day(&mut day_subscription_budget, existing.amount, &mut rng);
                let paid_at = paid_at_for_day(month_start, paid_day, &mut rng);
                let period_end = paid_at + subscription_period_seconds(existing.interval_unit);

                month_subscription_revenue += existing.amount;
                pending_renewals.push(PendingRenewal {
                    subscription: existing,
                    paid_day,
                    paid_at,
                    period_end,
                });
            } else {
                carried_subscriptions.push(existing);
            }
        }

        println!(
            "generate: month {:>2} — filling subscription revenue \
             (${:.2} / ${:.2}, {} active subs)…",
            month_idx + 1,
            month_subscription_revenue as f64 / 1000.0,
            *target_subscription_revenue as f64 / 1000.0,
            carried_subscriptions.len() + pending_renewals.len(),
        );
        let mut pending_new_subscriptions = Vec::new();
        while month_subscription_revenue < *target_subscription_revenue {
            let remaining = *target_subscription_revenue - month_subscription_revenue;
            let monthly_amount = pick_subscription_amount(remaining, &mut rng);
            let (interval_unit, amount) = subscription_plan_for_new(
                remaining,
                monthly_amount,
                annual_subscription_share,
                &mut rng,
            );
            let paid_day = schedule_payment_day(&mut day_subscription_budget, amount, &mut rng);
            let paid_at = paid_at_for_day(month_start, paid_day, &mut rng);
            let period_end = paid_at + subscription_period_seconds(interval_unit);

            day_conversion_counts[paid_day] += 1;
            month_subscription_revenue += amount;
            pending_new_subscriptions.push(PendingNewSubscription {
                amount,
                monthly_amount,
                interval_unit,
                paid_day,
                paid_at,
                period_end,
            });
        }

        let mut pending_single_payments = Vec::with_capacity(single_amounts.len());
        for amount in single_amounts {
            let paid_day = schedule_payment_day(&mut day_single_budget, amount, &mut rng);
            let paid_at = paid_at_for_day(month_start, paid_day, &mut rng);

            day_conversion_counts[paid_day] += 1;
            pending_single_payments.push(PendingSinglePayment {
                amount,
                paid_day,
                paid_at,
            });
        }

        let total_conversions: usize = day_conversion_counts.iter().sum();
        let visitor_jitter = rng.gen_range(1.0 - VISITOR_MONTH_JITTER..=1.0 + VISITOR_MONTH_JITTER);
        let visitor_target = (total_conversions as f64 * VISITORS_PER_CONVERSION * visitor_jitter)
            .round()
            .max(total_conversions as f64) as usize;
        let day_visitor_counts = allocate_visitors_by_daily_conversions(
            visitor_target,
            &day_conversion_counts,
            &day_shape,
            &mut rng,
        );
        println!(
            "generate: month {:>2} — simulating {visitor_target} visitor sessions \
             from {total_conversions} conversions (subscription target ${:.2})…",
            month_idx + 1,
            *target_subscription_revenue as f64 / 1000.0,
        );

        let mut month_first_pages: Vec<i64> = Vec::with_capacity(visitor_target);
        let mut month_first_pages_by_day: Vec<FirstPagesDay> = (0..DAYS_PER_MONTH)
            .map(|_| FirstPagesDay::default())
            .collect();
        let mut month_all_pages: Vec<i64> = Vec::with_capacity(visitor_target * 3);
        let mut generated_timestamp_ids = HashSet::with_capacity(visitor_target * 3);

        let mut visitor_idx = 0_usize;
        for (day_idx, day_visitors) in day_visitor_counts.iter().enumerate() {
            let day_start = month_start + day_idx as i64 * DAY_SECONDS;
            for _ in 0..*day_visitors {
                visitor_idx += 1;
                if visitor_idx > 1 && visitor_idx % VISITOR_PROGRESS_INTERVAL == 0 {
                    println!(
                        "generate: month {:>2} — visitors {visitor_idx}/{visitor_target} \
                         ({:.0}%)",
                        month_idx + 1,
                        (visitor_idx as f64 / visitor_target as f64) * 100.0,
                    );
                }
                let session_start = day_start + rng.gen_range(0..(DAY_SECONDS - 1));
                let visitor_timestamp_id =
                    make_unique_timestamp_id(session_start, &mut generated_timestamp_ids, &mut rng);
                let first_client_page_id = make_96bit_id(&mut rng);

                month_first_pages.push(visitor_timestamp_id);
                month_all_pages.push(visitor_timestamp_id);

                let channel = pick_landing_channel(&mut rng);
                let (referrer_id, landing_query) =
                    pick_landing_attribution(channel, &referrer_ids, &referrer_pickers, &mut rng)?;
                let landing_query_map = landing_query
                    .as_deref()
                    .and_then(|query| serde_json::from_str::<Value>(query).ok())
                    .and_then(|value| value.as_object().cloned());
                let referrer_hostname = match referrer_id {
                    Some(id) => Some(referrer_hostname(&conn, id)?),
                    None => None,
                };
                let referrer_type = if landing_query_map
                    .as_ref()
                    .and_then(click_id_network_from_map)
                    .is_some()
                {
                    Some(ReferrerType::Campaign)
                } else {
                    classify_referrer_type(referrer_hostname.as_deref(), landing_query_map.as_ref())
                };
                let has_campaign = landing_query_will_create_campaign(landing_query.as_deref());
                let source_group =
                    source_group_for_landing(has_campaign, referrer_id.is_some(), referrer_type);
                // is_google means organic Google Search (CHECK: is_google => referrer_type=search).
                let is_google = referrer_type == Some(ReferrerType::Search)
                    && referrer_hostname.as_deref().is_some_and(referrer_is_google);
                month_first_pages_by_day[day_idx].push(
                    visitor_timestamp_id,
                    source_group,
                    is_google,
                );
                let (mut current_path_idx, landing_path) = landing_path_pickers.pick(
                    source_group,
                    referrer_id,
                    &referrer_ids,
                    &path_ids,
                    &mut rng,
                );
                let landing_metadata = maybe_landing_metadata(&mut rng)?;
                let merged_landing_metadata =
                    revscope::merge_query_into_metadata(landing_query.as_deref(), landing_metadata);
                let (landing_campaign_id, landing_page_metadata, landing_click_id) =
                    if let Some(metadata_json) = merged_landing_metadata.as_deref() {
                        split_landing_attribution(&conn, metadata_json, session_start)?
                    } else {
                        (None, None, None)
                    };
                let session_shape = pick_non_conversion_session_shape(&mut rng);
                let (landing_duration_seconds, landing_scrolled_percent, extra_pages) =
                    match session_shape {
                        NonConversionSessionShape::Bounce => {
                            (rng.gen_range(2..=9), rng.gen_range(0..=30), 0_usize)
                        }
                        NonConversionSessionShape::Short => (
                            rng.gen_range(8..=90),
                            rng.gen_range(10..=70),
                            if rng.gen_bool(0.4) { 1 } else { 0 },
                        ),
                        NonConversionSessionShape::Medium => (
                            rng.gen_range(30..=300),
                            rng.gen_range(20..=100),
                            rng.gen_range(1..=4),
                        ),
                    };
                let mut session_duration_seconds = landing_duration_seconds;
                let mut session_page_count = 1_i64;
                let mut session_pages: Vec<(i64, i64)> =
                    vec![(visitor_timestamp_id, landing_duration_seconds)];
                let mut home_page = if current_path_idx == PATH_HOME {
                    Some((visitor_timestamp_id, landing_duration_seconds))
                } else {
                    None
                };

                insert_pageview(
                    &conn,
                    visitor_timestamp_id,
                    visitor_timestamp_id,
                    None,
                    landing_path,
                    landing_page_metadata.as_deref(),
                    landing_duration_seconds,
                    landing_scrolled_percent,
                )?;
                conn.execute(
                    "INSERT INTO client_page_ids_map (visitor_timestamp_id, page_id)
                     VALUES (?1, ?2)",
                    params![visitor_timestamp_id, first_client_page_id],
                )?;
                if !matches!(session_shape, NonConversionSessionShape::Bounce) {
                    maybe_emit_outbound_click_for_path(
                        &conn,
                        &event_name_ids,
                        current_path_idx,
                        visitor_timestamp_id,
                        landing_duration_seconds,
                        &mut generated_timestamp_ids,
                        &mut rng,
                    )?;
                }

                let (timezone, language, country, city, os, os_version, browser_family, device) =
                    random_device_identity(
                        &locale_pickers,
                        &os_pickers,
                        &browser_pickers,
                        &mut rng,
                    );
                let (screen_w, screen_h, viewport_w, viewport_h) =
                    random_screen_profile(device, &mut rng);
                conn.execute(
                    "INSERT INTO landings (
                        visitor_timestamp_id, session_duration_seconds, session_page_count,
                        referrer, campaign, click_id, referrer_type, is_google,
                        timezone, language, country, city, tracker_version, os, os_version,
                        browser_family, device,
                        screen_width, screen_height, viewport_width, viewport_height
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)",
                    params![
                        visitor_timestamp_id,
                        session_duration_seconds,
                        session_page_count,
                        referrer_id,
                        landing_campaign_id,
                        landing_click_id,
                        referrer_type.map(ReferrerType::as_str),
                        i64::from(is_google),
                        timezone,
                        language,
                        country,
                        city,
                        2.7_f64,
                        os,
                        os_version,
                        browser_family,
                        device,
                        screen_w,
                        screen_h,
                        viewport_w,
                        viewport_h,
                    ],
                )?;

                let mut previous_timestamp_id = visitor_timestamp_id;
                for extra_idx in 0..extra_pages {
                    let client_page_id = make_96bit_id(&mut rng);
                    let page_timestamp_seconds = (session_start
                        + rng.gen_range(5..=(extra_idx as i64 + 1) * 180))
                    .min(month_end);
                    let page_timestamp_id = make_unique_timestamp_id(
                        page_timestamp_seconds,
                        &mut generated_timestamp_ids,
                        &mut rng,
                    );
                    apply_pageview_duration_cap(
                        &conn,
                        previous_timestamp_id,
                        page_timestamp_id,
                        &mut session_duration_seconds,
                        &mut session_pages,
                    )?;
                    month_all_pages.push(page_timestamp_id);
                    let (next_path_idx, path_id) =
                        pick_next_path(current_path_idx, &path_ids, &mut rng);
                    current_path_idx = next_path_idx;
                    let page_duration_seconds = match session_shape {
                        NonConversionSessionShape::Bounce => 0,
                        NonConversionSessionShape::Short => rng.gen_range(5..=45),
                        NonConversionSessionShape::Medium => rng.gen_range(10..=180),
                    };
                    let page_scrolled_percent = rng.gen_range(0..=100);
                    let page_metadata = maybe_page_metadata(&mut rng)?;
                    insert_pageview(
                        &conn,
                        page_timestamp_id,
                        visitor_timestamp_id,
                        Some(previous_timestamp_id),
                        path_id,
                        page_metadata.as_deref(),
                        page_duration_seconds,
                        page_scrolled_percent,
                    )?;
                    conn.execute(
                        "INSERT INTO client_page_ids_map (visitor_timestamp_id, page_id)
                         VALUES (?1, ?2)",
                        params![page_timestamp_id, client_page_id],
                    )?;
                    session_duration_seconds += page_duration_seconds;
                    session_page_count += 1;
                    session_pages.push((page_timestamp_id, page_duration_seconds));
                    if current_path_idx == PATH_HOME {
                        home_page = Some((page_timestamp_id, page_duration_seconds));
                    }
                    if !matches!(session_shape, NonConversionSessionShape::Bounce) {
                        maybe_emit_outbound_click_for_path(
                            &conn,
                            &event_name_ids,
                            current_path_idx,
                            page_timestamp_id,
                            page_duration_seconds,
                            &mut generated_timestamp_ids,
                            &mut rng,
                        )?;
                    }

                    previous_timestamp_id = page_timestamp_id;
                }

                conn.execute(
                    "UPDATE landings
                     SET session_duration_seconds = ?2,
                         session_page_count = ?3
                     WHERE visitor_timestamp_id = ?1",
                    params![
                        visitor_timestamp_id,
                        session_duration_seconds,
                        session_page_count,
                    ],
                )?;

                let max_events = match session_shape {
                    NonConversionSessionShape::Bounce => 0,
                    NonConversionSessionShape::Short => {
                        if rng.gen_bool(0.12) {
                            1
                        } else {
                            0
                        }
                    }
                    NonConversionSessionShape::Medium => {
                        if rng.gen_bool(0.18) {
                            1
                        } else {
                            0
                        }
                    }
                };
                if let Some((home_page_timestamp_id, home_duration_seconds)) = home_page {
                    if !matches!(session_shape, NonConversionSessionShape::Bounce)
                        && rng.gen_bool(HOME_FUNNEL_ATTEMPT_PROB)
                    {
                        emit_home_funnel_events(
                            &conn,
                            &event_name_ids,
                            home_page_timestamp_id,
                            home_duration_seconds,
                            false,
                            &mut generated_timestamp_ids,
                            &mut rng,
                        )?;
                    }
                }
                let eligible_session_pages: Vec<(i64, i64)> = session_pages
                    .into_iter()
                    .filter(|(_, duration_seconds)| {
                        duration_seconds * NANOS_PER_SECOND > NANOS_PER_SECOND + 1
                    })
                    .collect();
                for _ in 0..max_events {
                    let Some(&(page_timestamp_id, duration_seconds)) =
                        eligible_session_pages.choose(&mut rng)
                    else {
                        break;
                    };
                    let Some(event_timestamp_id) = make_event_timestamp_id(
                        page_timestamp_id,
                        duration_seconds,
                        None,
                        &mut generated_timestamp_ids,
                        &mut rng,
                    ) else {
                        continue;
                    };
                    conn.execute(
                        "INSERT INTO events (timestamp_id, event_name_id, page_timestamp_id, metadata)
                         VALUES (?1, ?2, ?3, jsonb(?4))",
                        params![
                            event_timestamp_id,
                            event_name_ids[VISITOR_SESSION_EVENT_INDEX],
                            page_timestamp_id,
                            event_metadata(&mut rng)?,
                        ],
                    )?;
                }
            }
        }
        println!(
            "generate: month {:>2} — visitor sessions done ({visitor_target} sessions, \
             {} page ids tracked)",
            month_idx + 1,
            month_all_pages.len(),
        );

        for cancellation in pending_cancellations {
            if cancellation.emit_cancel_flow {
                emit_cancel_flow_opened(
                    &conn,
                    &path_ids,
                    &event_name_ids,
                    cancellation.visitor_timestamp_id,
                    cancellation.last_paid_at,
                    cancellation.canceled_at,
                    &mut generated_timestamp_ids,
                    &mut rng,
                )?;
            }
            conn.execute(
                "UPDATE subscriptions
                 SET status = 'canceled', canceled_at = ?1, ended_at = ?1, current_period_end = ?1
                 WHERE id = ?2",
                params![cancellation.canceled_at, cancellation.subscription_id],
            )?;
            append_subscription_version(
                &conn,
                cancellation.subscription_id,
                SubscriptionVersion {
                    effective_from: cancellation.canceled_at,
                    change_kind: "canceled",
                    status: "canceled",
                    plan_name: plan_tier_for_amount(cancellation.monthly_amount),
                    plan_amount: cancellation.amount,
                    mrr_amount: cancellation.monthly_amount,
                    interval_unit: cancellation.interval_unit,
                    canceled_at: Some(cancellation.canceled_at),
                    ended_at: Some(cancellation.canceled_at),
                },
            )?;
        }

        active_subscriptions = carried_subscriptions;
        for pending in pending_renewals {
            let fee = ((pending.subscription.amount as f64) * 0.029).round() as i64 + 300;
            let visitor_timestamp_id = choose_first_page_for_day(
                &month_first_pages_by_day,
                &month_first_pages,
                pending.paid_day,
                &mut rng,
            );
            let billing_cycle = subscription_billing_cycle(pending.subscription.interval_unit);

            insert_payment(
                &conn,
                &mut payment_ids,
                Some(pending.subscription.id),
                Some(pending.subscription.interval_unit),
                Some(visitor_timestamp_id),
                &pending.subscription.provider_customer_id,
                &pending.subscription.customer_email,
                "subscription",
                pending.subscription.amount,
                Some(fee),
                Some(pending.subscription.amount - fee),
                pending.paid_at,
                json!({
                    "plan": "renewal",
                    "billing_cycle": billing_cycle,
                    "product": "acme-analytics"
                })
                .to_string(),
                &mut rng,
            )?;

            conn.execute(
                "UPDATE subscriptions
                 SET current_period_start = ?1, current_period_end = ?2, interval_unit = ?3
                 WHERE id = ?4",
                params![
                    pending.paid_at,
                    pending.period_end,
                    pending.subscription.interval_unit,
                    pending.subscription.id
                ],
            )?;

            active_subscriptions.push(ActiveSubscription {
                current_period_end: pending.period_end,
                last_paid_at: pending.paid_at,
                ..pending.subscription
            });
        }

        let new_subscriptions_this_month = pending_new_subscriptions.len();
        let conversion_amounts: Vec<i64> = pending_new_subscriptions
            .iter()
            .map(|pending| pending.amount)
            .chain(pending_single_payments.iter().map(|pending| pending.amount))
            .collect();
        let mut conversion_groups =
            allocate_conversion_source_groups(&conversion_amounts, &mut rng).into_iter();
        let mut next_conversion_group = || conversion_groups.next().unwrap_or(SourceGroup::Direct);
        for pending in pending_new_subscriptions {
            let provider_customer_id = format!("cus_acme_{:07}", created_subscription_seq);
            let customer_email = format!("user{:07}@acme-example.com", created_subscription_seq);
            let provider_subscription_id = format!("sub_acme_{:07}", created_subscription_seq);
            let billing_cycle = subscription_billing_cycle(pending.interval_unit);

            conn.execute(
                "INSERT INTO subscriptions (
                    provider, provider_subscription_id, provider_customer_id, customer_email,
                    status, currency, plan_name, quantity, plan_amount, mrr_amount,
                    interval_unit, current_period_start, current_period_end, started_at, ended_at,
                    canceled_at, metadata
                 ) VALUES (
                    'stripe', ?1, ?2, ?3,
                    'active', 'USD', ?4, 1, ?5, ?6,
                    ?7, ?8, ?9, ?10, NULL,
                    NULL, ?11
                 )",
                params![
                    provider_subscription_id,
                    provider_customer_id.clone(),
                    customer_email.clone(),
                    plan_tier_for_amount(pending.monthly_amount),
                    pending.amount,
                    pending.monthly_amount,
                    pending.interval_unit,
                    pending.paid_at,
                    pending.period_end,
                    pending.paid_at,
                    json!({
                        "product": "acme-analytics",
                        "tier": plan_tier_for_amount(pending.monthly_amount),
                        "billing_cycle": billing_cycle,
                        "seat_count": rng.gen_range(1..=24),
                        "source": "self-serve"
                    })
                    .to_string(),
                ],
            )?;
            let subscription_id = last_insert_rowid(&conn)?;
            append_subscription_version(
                &conn,
                subscription_id,
                SubscriptionVersion {
                    effective_from: pending.paid_at,
                    change_kind: "created",
                    status: "active",
                    plan_name: plan_tier_for_amount(pending.monthly_amount),
                    plan_amount: pending.amount,
                    mrr_amount: pending.monthly_amount,
                    interval_unit: pending.interval_unit,
                    canceled_at: None,
                    ended_at: None,
                },
            )?;

            let fee = ((pending.amount as f64) * 0.029).round() as i64 + 300;
            let visitor_timestamp_id = choose_first_page_for_conversion(
                &month_first_pages_by_day,
                pending.paid_day,
                pending.paid_at,
                next_conversion_group(),
                &mut rng,
            );
            extend_conversion_session(&conn, visitor_timestamp_id, pending.paid_at, &mut rng)?;
            insert_checkout_before_purchase(
                &conn,
                &path_ids,
                &event_name_ids,
                visitor_timestamp_id,
                pending.paid_at,
                &mut generated_timestamp_ids,
                &mut rng,
            )?;
            insert_payment(
                &conn,
                &mut payment_ids,
                Some(subscription_id),
                Some(pending.interval_unit),
                Some(visitor_timestamp_id),
                &provider_customer_id,
                &customer_email,
                "subscription",
                pending.amount,
                Some(fee),
                Some(pending.amount - fee),
                pending.paid_at,
                json!({
                    "plan": "new",
                    "billing_cycle": billing_cycle,
                    "trial_days": if rng.gen_bool(0.2) { 14 } else { 0 },
                    "product": "acme-analytics"
                })
                .to_string(),
                &mut rng,
            )?;
            generate_post_conversion_activity(
                &conn,
                &path_ids,
                &post_conversion_paths,
                &event_name_ids,
                visitor_timestamp_id,
                pending.paid_at,
                now,
                true,
                &mut generated_timestamp_ids,
                &mut rng,
            )?;

            active_subscriptions.push(ActiveSubscription {
                id: subscription_id,
                amount: pending.amount,
                interval_unit: pending.interval_unit,
                current_period_end: pending.period_end,
                provider_customer_id,
                customer_email,
                visitor_timestamp_id,
                last_paid_at: pending.paid_at,
            });
            created_subscription_seq += 1;
        }
        if new_subscriptions_this_month > 0 {
            println!(
                "generate: month {:>2} — added {new_subscriptions_this_month} new subscriptions",
                month_idx + 1,
            );
        }

        println!(
            "generate: month {:>2} — inserting {single_payment_count} one-off payments…",
            month_idx + 1,
        );
        for pending in pending_single_payments {
            let fee = ((pending.amount as f64) * 0.029).round() as i64 + 300;
            let customer_idx = rng.gen_range(50_000..=99_999);
            let provider_customer_id = format!("cus_single_{customer_idx}");
            let customer_email = format!("consulting{customer_idx}@acme-example.com");
            let visitor_timestamp_id = choose_first_page_for_conversion(
                &month_first_pages_by_day,
                pending.paid_day,
                pending.paid_at,
                next_conversion_group(),
                &mut rng,
            );
            extend_conversion_session(&conn, visitor_timestamp_id, pending.paid_at, &mut rng)?;
            insert_checkout_before_purchase(
                &conn,
                &path_ids,
                &event_name_ids,
                visitor_timestamp_id,
                pending.paid_at,
                &mut generated_timestamp_ids,
                &mut rng,
            )?;
            insert_payment(
                &conn,
                &mut payment_ids,
                None,
                None,
                Some(visitor_timestamp_id),
                &provider_customer_id,
                &customer_email,
                "single",
                pending.amount,
                Some(fee),
                Some(pending.amount - fee),
                pending.paid_at,
                json!({
                    "sku": if rng.gen_bool(0.5) { "onboarding" } else { "consulting" },
                    "department": "sales"
                })
                .to_string(),
                &mut rng,
            )?;
            generate_post_conversion_activity(
                &conn,
                &path_ids,
                &post_conversion_paths,
                &event_name_ids,
                visitor_timestamp_id,
                pending.paid_at,
                now,
                false,
                &mut generated_timestamp_ids,
                &mut rng,
            )?;
        }

        for _ in 0..MIN_REFUNDS_PER_MONTH {
            insert_risk_payment(
                &conn,
                &mut payment_ids,
                &active_subscriptions,
                &month_first_pages,
                month_start,
                month_end,
                "refund",
                &mut rng,
            )?;
        }
        for _ in 0..MIN_CHARGEBACKS_PER_MONTH {
            insert_risk_payment(
                &conn,
                &mut payment_ids,
                &active_subscriptions,
                &month_first_pages,
                month_start,
                month_end,
                "chargeback",
                &mut rng,
            )?;
        }

        println!(
            "month {:>2}: target ${:.2}, generated ${:.2}, visitors {}",
            month_idx + 1,
            *target_subscription_revenue as f64 / 1000.0,
            month_subscription_revenue as f64 / 1000.0,
            visitor_target
        );
    }

    let start_day = (start_ts / DAY_SECONDS) * DAY_SECONDS;
    let end_day = (now / DAY_SECONDS) * DAY_SECONDS;
    println!("generate: populating search_queries ({SEARCH_QUERY_COUNT} queries)…");
    populate_search_queries(&conn, &path_ids, start_day, end_day, &mut rng)?;

    println!("generate: populating crawler_views_by_hour…");
    let crawler_path_ids = upsert_crawler_paths(&conn)?;
    populate_crawler_views(&conn, &crawler_path_ids, start_ts, now, &mut rng)?;

    println!("generate: inserting home funnel checkout drop-offs…");
    let mut dropoff_timestamp_ids = HashSet::new();
    insert_home_funnel_checkout_dropoffs(
        &conn,
        &path_ids,
        &event_name_ids,
        now,
        &mut dropoff_timestamp_ids,
        &mut rng,
    )?;

    println!("generate: creating home landing conversion funnel…");
    setup_home_conversion_funnel(&conn, &path_ids, &event_name_ids)?;

    println!("generate: committing transaction…");
    conn.execute_batch("COMMIT;")?;
    println!("generate: checkpointing WAL (FULL)…");
    let raw: &rusqlite::Connection = &conn;
    let busy: i64 = raw.query_row("PRAGMA wal_checkpoint(FULL)", [], |row| row.get(0))?;
    if busy != 0 {
        return Err(format!("wal_checkpoint(FULL) returned busy={busy}").into());
    }
    drop(conn);
    println!("Acme dataset generated successfully.");
    Ok(())
}
fn upsert_paths(conn: &Connection) -> Result<Vec<i64>, Box<dyn Error>> {
    let internal_paths = [
        ("/", "acme.test"),
        ("/pricing", "acme.test"),
        ("/features", "acme.test"),
        ("/blog", "acme.test"),
        ("/blog/saas-metrics", "acme.test"),
        ("/blog/churn-playbook", "acme.test"),
        ("/docs", "app.acme.test"),
        ("/docs/getting-started", "app.acme.test"),
        ("/docs/api", "app.acme.test"),
        ("/login", "app.acme.test"),
        ("/signup", "app.acme.test"),
        ("/dashboard", "app.acme.test"),
        ("/checkout", "app.acme.test"),
        ("/forgot-password", "app.acme.test"),
    ];

    let mut ids = Vec::with_capacity(internal_paths.len());
    for (path, host) in internal_paths {
        conn.execute(
            "INSERT OR IGNORE INTO paths (path, hostname) VALUES (?1, ?2)",
            params![path, host],
        )?;

        let id: i64 = conn.query_row(
            "SELECT id FROM paths
             WHERE path = ?1
               AND hostname = ?2",
            params![path, host],
            |row| row.get(0),
        )?;
        ids.push(id);
    }

    Ok(ids)
}

fn upsert_crawler_paths(conn: &Connection) -> Result<Vec<i64>, Box<dyn Error>> {
    let crawler_paths = [
        ("/", "acme.test"),
        ("/pricing", "acme.test"),
        ("/features", "acme.test"),
        ("/blog", "acme.test"),
        ("/blog/saas-metrics", "acme.test"),
        ("/blog/churn-playbook", "acme.test"),
        ("/docs", "app.acme.test"),
        ("/docs/getting-started", "app.acme.test"),
        ("/robots.txt", "acme.test"),
        ("/sitemap.xml", "acme.test"),
        ("/.well-known/security.txt", "acme.test"),
    ];
    let mut ids = Vec::with_capacity(crawler_paths.len());
    for (path, host) in crawler_paths {
        ids.push(upsert_crawler_path(conn, path, host)?);
    }
    Ok(ids)
}

fn upsert_referrers(conn: &Connection) -> Result<Vec<i64>, Box<dyn Error>> {
    // Referrers are hostnames only (no scheme, www., or path).
    let referrers: &[&str] = &[
        "google.com",
        "bing.com",
        "duckduckgo.com",
        "search.brave.com",
        "linkedin.com",
        "x.com",
        "reddit.com",
        "youtube.com",
        "facebook.com",
        "instagram.com",
        "tiktok.com",
        "news.ycombinator.com",
        "producthunt.com",
        "indiehackers.com",
        "g2.com",
        "capterra.com",
        "comparison.blog",
        "agency.example.com",
        "newsletter.growthweekly.io",
        "newsletter.saasops.org",
        "partners.affiliatly.com",
        "partnerstack.com",
        "ads.acme.test",
        "googleads.g.doubleclick.net",
        "googlesyndication.com",
        "l.facebook.com",
        "l.instagram.com",
        "chatgpt.com",
        "claude.ai",
        "gemini.google.com",
        "grok.com",
        "perplexity.ai",
        "copilot.microsoft.com",
        "phind.com",
        "poe.com",
        "you.com",
        "openai.com",
    ];

    let mut ids = Vec::with_capacity(referrers.len());
    for value in referrers {
        conn.execute(
            "INSERT OR IGNORE INTO referrers (referrer) VALUES (?1)",
            params![value],
        )?;
        let id: i64 = conn.query_row(
            "SELECT id FROM referrers WHERE referrer = ?1",
            params![value],
            |row| row.get(0),
        )?;
        ids.push(id);
    }
    Ok(ids)
}

fn referrer_hostname(conn: &Connection, referrer_id: i64) -> Result<String, Box<dyn Error>> {
    conn.query_row(
        "SELECT referrer FROM referrers WHERE id = ?1",
        params![referrer_id],
        |row| row.get(0),
    )
    .map_err(|err| err.into())
}

fn upsert_event_names(conn: &Connection) -> Result<Vec<i64>, Box<dyn Error>> {
    let event_names = [
        "view_problem",
        "view_solution",
        "view_pricing",
        "cta_click",
        "signup_started",
        "signup_completed",
        "plan_selected",
        "checkout_started",
        "feature_tour_viewed",
        "integration_connected",
        "team_invite_sent",
        "dashboard_exported",
        "webhook_created",
        "trial_extended",
        "cancel_flow_opened",
        "revscope-outbound-click",
    ];

    let mut ids = Vec::with_capacity(event_names.len());
    for value in event_names {
        conn.execute(
            "INSERT OR IGNORE INTO event_names (name) VALUES (?1)",
            params![value],
        )?;
        let id: i64 = conn.query_row(
            "SELECT id FROM event_names WHERE name = ?1",
            params![value],
            |row| row.get(0),
        )?;
        ids.push(id);
    }
    Ok(ids)
}

fn last_insert_rowid(conn: &Connection) -> Result<i64, Box<dyn Error>> {
    Ok(conn.last_insert_rowid())
}

/// Appends a subscription history version, closing whichever version is open.
///
/// The fingerprint a real sync hashes from the provider payload has no analogue
/// here, so versions carry a synthetic one; it only guards against re-appending
/// unchanged state on resync.
fn append_subscription_version(
    conn: &Connection,
    subscription_id: i64,
    version: SubscriptionVersion,
) -> Result<(), Box<dyn Error>> {
    let open: Option<(i64, i64, Option<i64>)> = conn
        .query_row(
            "SELECT id, version, mrr_amount
             FROM subscription_history
             WHERE subscription_id = ?1 AND effective_to IS NULL",
            params![subscription_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let (version_number, prev_mrr_amount) = match open {
        Some((open_id, open_version, open_mrr_amount)) => {
            conn.execute(
                "UPDATE subscription_history SET effective_to = ?2 WHERE id = ?1",
                params![open_id, version.effective_from],
            )?;
            (open_version + 1, open_mrr_amount)
        }
        None => (1, None),
    };

    conn.execute(
        "INSERT INTO subscription_history (
            subscription_id, version, effective_from, observed_at, change_kind, status,
            plan_name, quantity, currency, plan_amount, mrr_amount, prev_mrr_amount,
            interval_unit, interval_count, canceled_at, ended_at, fingerprint
         ) VALUES (
            ?1, ?2, ?3, ?3, ?4, ?5,
            ?6, 1, 'USD', ?7, ?8, ?9,
            ?10, 1, ?11, ?12, ?13
         )",
        params![
            subscription_id,
            version_number,
            version.effective_from,
            version.change_kind,
            version.status,
            version.plan_name,
            version.plan_amount,
            version.mrr_amount,
            prev_mrr_amount,
            version.interval_unit,
            version.canceled_at,
            version.ended_at,
            format!("demo-{subscription_id}-{version_number}"),
        ],
    )?;
    Ok(())
}

/// Organic search landing pages (indices into `upsert_paths` / `path_ids`).
const SEARCH_PATH_INDICES: [usize; 6] = [0, 1, 2, 3, 4, 5];

/// Organic query intent mapped to a marketing path index (0 = `/` … 5 = churn playbook).
struct SearchQuerySeed {
    query: &'static str,
    path_idx: usize,
    position: i32,
    ctr: f64,
}

const SEARCH_QUERY_SEEDS: [SearchQuerySeed; 51] = [
    SearchQuerySeed {
        query: "acme analytics",
        path_idx: 0,
        position: 2,
        ctr: 0.18,
    },
    SearchQuerySeed {
        query: "acme revenue attribution",
        path_idx: 0,
        position: 3,
        ctr: 0.15,
    },
    SearchQuerySeed {
        query: "acme saas dashboard",
        path_idx: 0,
        position: 4,
        ctr: 0.12,
    },
    SearchQuerySeed {
        query: "acme growth analytics",
        path_idx: 0,
        position: 5,
        ctr: 0.10,
    },
    SearchQuerySeed {
        query: "acme analytics login",
        path_idx: 0,
        position: 6,
        ctr: 0.09,
    },
    SearchQuerySeed {
        query: "what is acme analytics",
        path_idx: 0,
        position: 7,
        ctr: 0.08,
    },
    SearchQuerySeed {
        query: "acme analytics reviews",
        path_idx: 0,
        position: 8,
        ctr: 0.07,
    },
    SearchQuerySeed {
        query: "acme analytics vs mixpanel",
        path_idx: 0,
        position: 9,
        ctr: 0.06,
    },
    SearchQuerySeed {
        query: "saas revenue analytics pricing",
        path_idx: 1,
        position: 3,
        ctr: 0.14,
    },
    SearchQuerySeed {
        query: "revenue attribution software cost",
        path_idx: 1,
        position: 4,
        ctr: 0.11,
    },
    SearchQuerySeed {
        query: "product analytics pricing comparison",
        path_idx: 1,
        position: 5,
        ctr: 0.10,
    },
    SearchQuerySeed {
        query: "stripe revenue dashboard price",
        path_idx: 1,
        position: 6,
        ctr: 0.09,
    },
    SearchQuerySeed {
        query: "mrr analytics tool pricing",
        path_idx: 1,
        position: 7,
        ctr: 0.08,
    },
    SearchQuerySeed {
        query: "subscription analytics pricing",
        path_idx: 1,
        position: 8,
        ctr: 0.07,
    },
    SearchQuerySeed {
        query: "b2b saas analytics plans",
        path_idx: 1,
        position: 9,
        ctr: 0.06,
    },
    SearchQuerySeed {
        query: "revenue attribution features",
        path_idx: 2,
        position: 4,
        ctr: 0.11,
    },
    SearchQuerySeed {
        query: "saas metrics dashboard features",
        path_idx: 2,
        position: 5,
        ctr: 0.10,
    },
    SearchQuerySeed {
        query: "cohort analysis for subscriptions",
        path_idx: 2,
        position: 6,
        ctr: 0.09,
    },
    SearchQuerySeed {
        query: "stripe webhook revenue tracking",
        path_idx: 2,
        position: 7,
        ctr: 0.08,
    },
    SearchQuerySeed {
        query: "utm free trial attribution",
        path_idx: 2,
        position: 8,
        ctr: 0.07,
    },
    SearchQuerySeed {
        query: "marketing channel roi analytics",
        path_idx: 2,
        position: 9,
        ctr: 0.06,
    },
    SearchQuerySeed {
        query: "first touch vs last touch attribution",
        path_idx: 2,
        position: 11,
        ctr: 0.05,
    },
    SearchQuerySeed {
        query: "saas analytics blog",
        path_idx: 3,
        position: 5,
        ctr: 0.10,
    },
    SearchQuerySeed {
        query: "how to measure saas revenue",
        path_idx: 3,
        position: 6,
        ctr: 0.09,
    },
    SearchQuerySeed {
        query: "startup revenue metrics guide",
        path_idx: 3,
        position: 7,
        ctr: 0.08,
    },
    SearchQuerySeed {
        query: "product led growth analytics",
        path_idx: 3,
        position: 8,
        ctr: 0.07,
    },
    SearchQuerySeed {
        query: "track trial to paid conversion",
        path_idx: 3,
        position: 9,
        ctr: 0.06,
    },
    SearchQuerySeed {
        query: "marketing funnel analytics saas",
        path_idx: 3,
        position: 11,
        ctr: 0.05,
    },
    SearchQuerySeed {
        query: "saas metrics that matter",
        path_idx: 4,
        position: 6,
        ctr: 0.09,
    },
    SearchQuerySeed {
        query: "net revenue retention benchmark",
        path_idx: 4,
        position: 7,
        ctr: 0.08,
    },
    SearchQuerySeed {
        query: "mrr vs arr reporting",
        path_idx: 4,
        position: 8,
        ctr: 0.07,
    },
    SearchQuerySeed {
        query: "expansion revenue analytics",
        path_idx: 4,
        position: 9,
        ctr: 0.06,
    },
    SearchQuerySeed {
        query: "saas kpi dashboard examples",
        path_idx: 4,
        position: 10,
        ctr: 0.05,
    },
    SearchQuerySeed {
        query: "revenue per visitor saas",
        path_idx: 4,
        position: 12,
        ctr: 0.04,
    },
    SearchQuerySeed {
        query: "reduce saas churn playbook",
        path_idx: 5,
        position: 7,
        ctr: 0.08,
    },
    SearchQuerySeed {
        query: "subscription churn analysis",
        path_idx: 5,
        position: 8,
        ctr: 0.07,
    },
    SearchQuerySeed {
        query: "churn rate formula saas",
        path_idx: 5,
        position: 9,
        ctr: 0.06,
    },
    SearchQuerySeed {
        query: "customer retention metrics b2b",
        path_idx: 5,
        position: 10,
        ctr: 0.05,
    },
    SearchQuerySeed {
        query: "voluntary vs involuntary churn",
        path_idx: 5,
        position: 11,
        ctr: 0.05,
    },
    SearchQuerySeed {
        query: "dunning recovery analytics",
        path_idx: 5,
        position: 13,
        ctr: 0.04,
    },
    SearchQuerySeed {
        query: "best revenue attribution for stripe",
        path_idx: 1,
        position: 4,
        ctr: 0.12,
    },
    SearchQuerySeed {
        query: "google analytics alternative saas",
        path_idx: 1,
        position: 5,
        ctr: 0.10,
    },
    SearchQuerySeed {
        query: "plg funnel analytics platform",
        path_idx: 2,
        position: 5,
        ctr: 0.10,
    },
    SearchQuerySeed {
        query: "attribution modeling for b2b saas",
        path_idx: 3,
        position: 6,
        ctr: 0.09,
    },
    SearchQuerySeed {
        query: "calculate ltv cac ratio dashboard",
        path_idx: 4,
        position: 7,
        ctr: 0.08,
    },
    SearchQuerySeed {
        query: "revscope analytics",
        path_idx: 0,
        position: 4,
        ctr: 0.11,
    },
    SearchQuerySeed {
        query: "self serve saas billing analytics",
        path_idx: 1,
        position: 6,
        ctr: 0.09,
    },
    SearchQuerySeed {
        query: "connect stripe to analytics",
        path_idx: 2,
        position: 7,
        ctr: 0.08,
    },
    SearchQuerySeed {
        query: "organic search revenue reporting",
        path_idx: 3,
        position: 8,
        ctr: 0.07,
    },
    SearchQuerySeed {
        query: "logo churn vs revenue churn",
        path_idx: 5,
        position: 9,
        ctr: 0.06,
    },
    SearchQuerySeed {
        query: "improve trial conversion rate saas",
        path_idx: 5,
        position: 10,
        ctr: 0.05,
    },
];

#[derive(Clone)]
struct SearchTermCombo {
    query: String,
    path_ids: Vec<i64>,
    /// [desktop, mobile] click share weights (tablets grouped with mobile).
    device_weights: [f64; 2],
    position: i32,
    peak_impressions: i32,
    ctr: f64,
}

const SEARCH_QUERY_PREFIXES: &[&str] = &["", "best ", "top ", "how to ", "what is "];
const SEARCH_QUERY_SUFFIXES: &[&str] = &[
    "",
    " tool",
    " software",
    " platform",
    " for saas",
    " for startups",
    " 2025",
    " guide",
];

struct SearchQuerySpec {
    query: String,
    path_idx: usize,
    position: i32,
    ctr: f64,
}

fn sample_pareto_weights(n: usize, alpha: f64, rng: &mut StdRng) -> Vec<f64> {
    let mut weights: Vec<f64> = (0..n)
        .map(|_| {
            let uniform = rng.gen_range(0.005..1.0_f64);
            uniform.powf(-1.0 / alpha)
        })
        .collect();
    let sum: f64 = weights.iter().sum();
    if sum <= 0.0 {
        return vec![1.0 / n as f64; n];
    }
    for weight in &mut weights {
        *weight /= sum;
    }
    weights
}

fn allocate_i32_by_weights(total: i32, weights: &[f64]) -> Vec<i32> {
    if total <= 0 {
        return vec![0; weights.len()];
    }
    let weight_sum: f64 = weights.iter().sum();
    let mut amounts: Vec<i32> = weights
        .iter()
        .map(|weight| ((total as f64) * weight / weight_sum).floor() as i32)
        .collect();
    let remainder = (total - amounts.iter().sum::<i32>()).max(0) as usize;
    let mut fractional: Vec<(usize, f64)> = weights
        .iter()
        .enumerate()
        .map(|(idx, weight)| {
            let exact = (total as f64) * weight / weight_sum;
            (idx, exact - amounts[idx] as f64)
        })
        .collect();
    fractional.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    for (idx, _) in fractional.into_iter().take(remainder) {
        amounts[idx] += 1;
    }
    amounts
}

/// Top seeded queries get extra landing paths so search-term cannibalization
/// highlights have something to flag after a generate run.
const FORCED_CANNIBALIZED_QUERY_COUNT: usize = 8;

fn build_search_term_path_ids(
    primary_path_id: i64,
    path_ids: &[i64],
    rng: &mut StdRng,
    force_cannibalized: bool,
) -> Vec<i64> {
    let target_count = if force_cannibalized {
        if rng.gen_bool(0.5) { 2 } else { 3 }
    } else if rng.gen_bool(0.82) {
        1
    } else if rng.gen_bool(0.85) {
        2
    } else {
        3
    };
    let mut chosen = vec![primary_path_id];
    let mut seen: HashSet<i64> = HashSet::from([primary_path_id]);
    let mut attempts = 0;

    while chosen.len() < target_count && attempts < SEARCH_PATH_INDICES.len() * 4 {
        attempts += 1;
        let idx = SEARCH_PATH_INDICES[rng.gen_range(0..SEARCH_PATH_INDICES.len())];
        let path_id = path_ids[idx];
        if seen.insert(path_id) {
            chosen.push(path_id);
        }
    }

    chosen
}

fn build_search_term_device_weights(rng: &mut StdRng) -> [f64; 2] {
    // [desktop, mobile]; not required to sum to 1 — allocate_i32_by_weights normalizes.
    [rng.gen_range(0.44..0.66), rng.gen_range(0.34..0.56)]
}

fn build_search_term_combos_with_alpha(
    path_ids: &[i64],
    alpha: f64,
    rng: &mut StdRng,
) -> Vec<SearchTermCombo> {
    let mut query_specs = Vec::with_capacity(SEARCH_QUERY_COUNT);
    let mut seen: HashSet<String> = HashSet::new();
    for seed in SEARCH_QUERY_SEEDS {
        if query_specs.len() >= SEARCH_QUERY_COUNT {
            break;
        }
        if seen.insert(seed.query.to_string()) {
            query_specs.push(SearchQuerySpec {
                query: seed.query.to_string(),
                path_idx: seed.path_idx,
                position: seed.position,
                ctr: seed.ctr,
            });
        }
    }
    let mut attempts = 0_usize;
    while query_specs.len() < SEARCH_QUERY_COUNT && attempts < SEARCH_QUERY_COUNT * 40 {
        attempts += 1;
        let seed = SEARCH_QUERY_SEEDS
            .choose(rng)
            .expect("search query seeds should not be empty");
        let prefix = SEARCH_QUERY_PREFIXES[rng.gen_range(0..SEARCH_QUERY_PREFIXES.len())];
        let suffix = SEARCH_QUERY_SUFFIXES[rng.gen_range(0..SEARCH_QUERY_SUFFIXES.len())];
        let query = format!("{prefix}{}{suffix}", seed.query);
        if query.len() > 120 || !seen.insert(query.clone()) {
            continue;
        }
        query_specs.push(SearchQuerySpec {
            query,
            path_idx: seed.path_idx,
            position: (seed.position + rng.gen_range(-1..=2)).clamp(1, 50),
            ctr: (seed.ctr * rng.gen_range(0.8..1.2)).clamp(0.02, 0.22),
        });
    }
    query_specs.truncate(SEARCH_QUERY_COUNT);

    let mut pareto_weights = sample_pareto_weights(query_specs.len(), alpha, rng);
    pareto_weights.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    let peak_impressions = allocate_i32_by_weights(SEARCH_PEAK_IMPRESSIONS_TOTAL, &pareto_weights);

    query_specs
        .into_iter()
        .zip(peak_impressions)
        .enumerate()
        .map(|(idx, (spec, peak_impressions))| {
            let path_id = path_ids[SEARCH_PATH_INDICES[spec.path_idx]];
            SearchTermCombo {
                query: spec.query,
                path_ids: build_search_term_path_ids(
                    path_id,
                    path_ids,
                    rng,
                    idx < FORCED_CANNIBALIZED_QUERY_COUNT,
                ),
                device_weights: build_search_term_device_weights(rng),
                position: spec.position,
                peak_impressions: peak_impressions.max(1),
                ctr: spec.ctr,
            }
        })
        .collect()
}

fn search_day_power_weight(day_idx: usize, total_days: usize, power: f64) -> f64 {
    if total_days <= 1 {
        return 1.0;
    }
    let t = (day_idx + 1) as f64 / total_days as f64;
    t.powf(power)
}

fn sample_search_path_weights(path_count: usize, rng: &mut StdRng) -> Vec<f64> {
    if path_count <= 1 {
        return vec![1.0];
    }
    // Flatter than the default Pareto so secondary paths keep a visible click
    // share instead of collapsing to zero after integer allocation.
    let mut weights = sample_pareto_weights(path_count, 0.9, rng);
    weights.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    weights.truncate(path_count);
    weights
}

fn insert_search_query_paths_for_day(
    conn: &Connection,
    query_id: i64,
    date: i64,
    clicks: i32,
    path_ids: &[i64],
    path_weights: &[f64],
) -> Result<usize, Box<dyn Error>> {
    if clicks <= 0 {
        return Ok(0);
    }

    let allocations = allocate_i32_by_weights(clicks, path_weights);
    let mut row_count = 0_usize;
    for (path_id, row_clicks) in path_ids.iter().zip(allocations) {
        if row_clicks <= 0 {
            continue;
        }
        conn.execute(
            "INSERT INTO search_query_paths_by_date (
                date, query_id, path_id, clicks
             ) VALUES (?1, ?2, ?3, ?4)",
            params![date, query_id, path_id, row_clicks],
        )?;
        row_count += 1;
    }

    Ok(row_count)
}

fn populate_search_queries(
    conn: &Connection,
    path_ids: &[i64],
    start_day: i64,
    end_day: i64,
    rng: &mut StdRng,
) -> Result<(), Box<dyn Error>> {
    let total_days = ((end_day - start_day) / DAY_SECONDS + 1).max(1) as usize;
    let power = rng.gen_range(SEARCH_DATE_POWER_MIN..=SEARCH_DATE_POWER_MAX);
    let alpha = rng.gen_range(SEARCH_PARETO_ALPHA_MIN..=SEARCH_PARETO_ALPHA_MAX);
    let combos = build_search_term_combos_with_alpha(path_ids, alpha, rng);
    let mut query_count = 0_usize;
    let mut by_date_row_count = 0_usize;
    let mut path_row_count = 0_usize;

    for combo in combos {
        conn.execute(
            "INSERT INTO search_queries (query) VALUES (?1)",
            params![&combo.query],
        )?;
        let query_id = last_insert_rowid(conn)?;
        query_count += 1;

        let path_weights = sample_search_path_weights(combo.path_ids.len(), rng);
        for day_idx in 0..total_days {
            let weight = search_day_power_weight(day_idx, total_days, power);
            let impressions =
                (combo.peak_impressions as f64 * weight * rng.gen_range(0.82..1.18)).round() as i32;
            if impressions < 1 {
                continue;
            }
            let clicks =
                ((impressions as f64) * combo.ctr * rng.gen_range(0.88..1.12)).round() as i32;
            let clicks = clicks.clamp(0, impressions);
            let position = (combo.position + rng.gen_range(-1..=1)).clamp(1, 50);
            let date = start_day + day_idx as i64 * DAY_SECONDS;
            let device_clicks = allocate_i32_by_weights(clicks, &combo.device_weights);
            let desktop_clicks = device_clicks[0];
            let mobile_clicks = device_clicks[1];

            conn.execute(
                "INSERT INTO search_queries_by_date (
                    date, query_id, position, impressions, clicks, mobile_clicks, desktop_clicks
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    date,
                    query_id,
                    position,
                    impressions,
                    clicks,
                    mobile_clicks,
                    desktop_clicks
                ],
            )?;
            by_date_row_count += 1;
            path_row_count += insert_search_query_paths_for_day(
                conn,
                query_id,
                date,
                clicks,
                &combo.path_ids,
                &path_weights,
            )?;
        }
    }

    println!(
        "generate: search_queries — {query_count} queries, {by_date_row_count} daily rows, \
         {path_row_count} path rows (Pareto α={alpha:.2}, date power {power:.2})",
    );
    Ok(())
}

const CRAWLER_USER_AGENTS: &[&str] = &[
    "mozilla/5.0 applewebkit/537.36 (khtml, like gecko); compatible; gptbot/1.4; +https://openai.com/gptbot",
    "mozilla/5.0 (iphone; cpu iphone os 18_4 like mac os x) applewebkit/605.1.15 (khtml, like gecko) version/19.3 mobile/15e148 safari/604.1; compatible; chatgpt-user/1.0; +https://openai.com/bot",
    "mozilla/5.0 (linux; android 15) applewebkit/537.36 (khtml, like gecko) chrome/126.0.0.0 mobile safari/537.36; compatible; chatgpt-user/1.0; +https://openai.com/bot",
    "mozilla/5.0 (macintosh; intel mac os x 10_15_7) applewebkit/537.36 (khtml, like gecko) chrome/131.0.0.0 safari/537.36; compatible; oai-searchbot/1.4; +https://openai.com/searchbot",
    "mozilla/5.0 applewebkit/537.36 (khtml, like gecko); compatible; claudebot/1.0; +claudebot@anthropic.com",
    "mozilla/5.0 applewebkit/537.36 (khtml, like gecko); compatible; claude-searchbot/1.0; +claudebot@anthropic.com",
    "mozilla/5.0 applewebkit/537.36 (khtml, like gecko); compatible; claude-user/1.0; +claudebot@anthropic.com",
    "mozilla/5.0 applewebkit/537.36 (khtml, like gecko); compatible; perplexitybot/1.0; +https://perplexity.ai/perplexitybot",
    "mozilla/5.0 applewebkit/537.36 (khtml, like gecko); compatible; perplexity-user/1.0; +https://perplexity.ai/perplexitybot",
    "mozilla/5.0 (compatible; bytespider; +https://zhanzhang.toutiao.com/)",
    "mozilla/5.0 (compatible; googlebot/2.1; +http://www.google.com/bot.html)",
    "mozilla/5.0 (compatible; bingbot/2.0; +http://www.bing.com/bingbot.htm)",
    "mozilla/5.0 (compatible; amazonbot/0.1; +https://developer.amazon.com/support/amazonbot)",
    "mozilla/5.0 (macintosh; intel mac os x 10_15_7) applewebkit/605.1.15 (khtml, like gecko) version/17.4 safari/605.1.15 (applebot/0.1; +http://www.apple.com/go/applebot)",
    "ccbot/2.0 (+https://commoncrawl.org/faq/)",
    "mozilla/5.0 (compatible; meta-externalagent/1.1; +https://developers.facebook.com/docs/sharing/webmasters/crawler)",
    "facebookexternalhit/1.1 (+http://www.facebook.com/externalhit_uatext.php) facebookbot",
    "mozilla/5.0 (compatible; baiduspider/2.0; +http://www.baidu.com/search/spider.html)",
    "mozilla/5.0 (compatible; google-cloudvertexbot/1.0; +https://developers.google.com/crawling)",
    "mozilla/5.0 (compatible; duckassistbot/1.0; +https://duckduckgo.com/duckassistbot.html)",
];

const HEADLESS_USER_AGENTS: &[&str] = &[
    "mozilla/5.0 (x11; linux x86_64) applewebkit/537.36 (khtml, like gecko) headlesschrome/126.0.6478.126 safari/537.36",
    "mozilla/5.0 (windows nt 10.0; win64; x64) applewebkit/537.36 (khtml, like gecko) headlesschrome/120.0.6099.109 safari/537.36",
    "mozilla/5.0 (x11; linux x86_64) applewebkit/537.36 (khtml, like gecko) headlesschrome/122.0.6261.94 safari/537.36",
];

fn populate_crawler_views(
    conn: &Connection,
    path_ids: &[i64],
    start_ts: i64,
    now: i64,
    rng: &mut StdRng,
) -> Result<(), Box<dyn Error>> {
    let mut crawler_ids = Vec::with_capacity(CRAWLER_USER_AGENTS.len());
    for useragent in CRAWLER_USER_AGENTS {
        crawler_ids.push(upsert_useragent(conn, useragent)?);
    }
    let mut headless_ids = Vec::with_capacity(HEADLESS_USER_AGENTS.len());
    for useragent in HEADLESS_USER_AGENTS {
        headless_ids.push(upsert_useragent(conn, useragent)?);
    }

    let start_hour = (start_ts / HOUR_SECONDS) * HOUR_SECONDS;
    let end_hour = (now / HOUR_SECONDS) * HOUR_SECONDS;
    let path_weights = sample_pareto_weights(path_ids.len().min(8), 1.1, rng);
    let top_paths: Vec<i64> = path_ids.iter().copied().take(path_weights.len()).collect();
    let ua_weights = sample_pareto_weights(crawler_ids.len(), 1.2, rng);
    let mut row_count = 0_usize;

    let mut hour = start_hour;
    while hour < end_hour {
        let hour_of_day = (hour / HOUR_SECONDS) % 24;
        let weeks = ((hour - start_hour) / (7 * DAY_SECONDS)) as f64;
        let growth = 1.0 + weeks * 0.015;
        let diurnal = if (7..=18).contains(&hour_of_day) {
            1.0
        } else {
            0.35
        };
        let active = ((crawler_ids.len() as f64) * 0.35 * diurnal).round() as usize;
        let active = active.clamp(2, crawler_ids.len());
        let mut order: Vec<usize> = (0..crawler_ids.len()).collect();
        order.shuffle(rng);
        order.truncate(active);

        for ua_idx in order {
            let base = (ua_weights[ua_idx] * 18.0 * growth * diurnal * rng.gen_range(0.7..1.3))
                .round() as i32;
            let hits = base.clamp(1, 80);
            let path_counts = allocate_i32_by_weights(hits, &path_weights);
            for (path_id, count) in top_paths.iter().zip(path_counts) {
                if count <= 0 {
                    continue;
                }
                conn.execute(
                    "INSERT INTO crawler_views_by_hour (
                        hour, source, useragent_id, path_id, estimated_count, verified
                     ) VALUES (?1, 1, ?2, ?3, ?4, 1)
                     ON CONFLICT(hour, source, useragent_id, path_id, verified) DO UPDATE SET
                        estimated_count = crawler_views_by_hour.estimated_count + excluded.estimated_count",
                    params![hour, crawler_ids[ua_idx], path_id, count],
                )?;
                row_count += 1;
            }
        }

        if rng.gen_bool(0.18) {
            let useragent_id = headless_ids[rng.gen_range(0..headless_ids.len())];
            let path_id = top_paths[rng.gen_range(0..top_paths.len())];
            conn.execute(
                "INSERT INTO crawler_views_by_hour (
                    hour, source, useragent_id, path_id, estimated_count, verified
                 ) VALUES (?1, 0, ?2, ?3, 1, 0)
                 ON CONFLICT(hour, source, useragent_id, path_id, verified) DO UPDATE SET
                    estimated_count = crawler_views_by_hour.estimated_count + 1",
                params![hour, useragent_id, path_id],
            )?;
            row_count += 1;
        }

        hour += HOUR_SECONDS;
    }

    println!("generate: crawler_views_by_hour — {row_count} upserts");
    Ok(())
}

/// Indices into `path_ids` aligned with `upsert_paths`.
const PATH_HOME: usize = 0;
const PATH_PRICING: usize = 1;
const PATH_FEATURES: usize = 2;
const PATH_BLOG: usize = 3;
const PATH_BLOG_SAAS: usize = 4;
const PATH_BLOG_CHURN: usize = 5;
const PATH_DOCS: usize = 6;
const PATH_DOCS_START: usize = 7;
const PATH_DOCS_API: usize = 8;
const PATH_LOGIN: usize = 9;
const PATH_SIGNUP: usize = 10;
const PATH_DASHBOARD: usize = 11;
const PATH_CHECKOUT: usize = 12;
const PATH_FORGOT_PASSWORD: usize = 13;

/// Landing pages: all paths except login, signup, checkout, and dashboard.
const LANDING_PATH_INDICES: [usize; 9] = [0, 1, 2, 3, 4, 5, 6, 7, 8];

/// Pareto shape for landing-page visit share when a source has multiple pages.
const LANDING_PARETO_ALPHA: f64 = 1.65;

/// Number of rows in `upsert_referrers`.
const REFERRER_COUNT: usize = 37;

/// Reachable next pages from each path (realistic in-site navigation).
fn next_path_indices(from: usize) -> &'static [usize] {
    match from {
        PATH_HOME => &[
            PATH_PRICING,
            PATH_FEATURES,
            PATH_BLOG,
            PATH_DOCS,
            PATH_LOGIN,
            PATH_SIGNUP,
            PATH_DASHBOARD,
            PATH_CHECKOUT,
        ],
        PATH_PRICING => &[PATH_HOME, PATH_FEATURES, PATH_CHECKOUT, PATH_SIGNUP],
        PATH_FEATURES => &[PATH_HOME, PATH_PRICING, PATH_CHECKOUT, PATH_BLOG],
        PATH_BLOG | PATH_BLOG_SAAS | PATH_BLOG_CHURN => {
            &[PATH_HOME, PATH_BLOG, PATH_BLOG_SAAS, PATH_BLOG_CHURN]
        }
        PATH_DOCS | PATH_DOCS_START | PATH_DOCS_API => {
            &[PATH_HOME, PATH_DOCS, PATH_DOCS_START, PATH_DOCS_API]
        }
        PATH_LOGIN => &[PATH_DASHBOARD, PATH_FORGOT_PASSWORD],
        PATH_SIGNUP => &[PATH_CHECKOUT, PATH_PRICING, PATH_HOME],
        PATH_DASHBOARD => &[PATH_HOME, PATH_PRICING, PATH_DOCS],
        PATH_CHECKOUT => &[PATH_DASHBOARD],
        PATH_FORGOT_PASSWORD => &[PATH_LOGIN, PATH_HOME],
        _ => &[PATH_HOME],
    }
}

/// Indices into `upsert_referrers` / `referrer_ids`.
const SEARCH_REFERRER_INDICES: [usize; 4] = [0, 1, 2, 3];
/// google.com / bing / duckduckgo / brave — Google ~95% of organic search traffic.
const SEARCH_REFERRER_WEIGHTS: [u32; 4] = [95, 2, 2, 1];
const GOOGLE_ADS_REFERRER_INDICES: [usize; 2] = [23, 24];
const GOOGLE_ADS_REFERRER_WEIGHTS: [u32; 2] = [4, 1];
const SOCIAL_ORGANIC_REFERRER_INDICES: [usize; 7] = [4, 5, 6, 7, 8, 9, 10];
const SOCIAL_ORGANIC_REFERRER_WEIGHTS: [u32; 7] = [3, 3, 3, 3, 3, 3, 3];
const PAID_SOCIAL_LINKEDIN_INDICES: [usize; 1] = [4];
const PAID_SOCIAL_X_INDICES: [usize; 1] = [5];
const META_ADS_REFERRER_INDICES: [usize; 2] = [25, 26];
/// Share of search-channel landings that are Google Ads (with campaign query params).
const SEARCH_GOOGLE_ADS_SHARE: f64 = 0.10;
/// Share of social-channel landings that are paid social (with campaign query params).
const SOCIAL_PAID_SHARE: f64 = 0.38;
/// Newsletter, affiliate partners, and the synthetic ads landing host.
const CAMPAIGN_REFERRER_INDICES: [usize; 5] = [18, 19, 20, 21, 22];
const CAMPAIGN_REFERRER_WEIGHTS: [u32; 5] = [3, 3, 3, 3, 8];
/// Review sites, communities, and editorial referrers (unclassified `referrer_type`).
const OTHER_REFERRER_INDICES: [usize; 7] = [11, 12, 13, 14, 15, 16, 17];
const OTHER_REFERRER_WEIGHTS: [u32; 7] = [3, 3, 3, 3, 3, 3, 3];
/// Indices into `upsert_referrers` for AI assistant hostnames (chatgpt … openai.com).
const AI_REFERRER_INDICES: [usize; 7] = [27, 28, 29, 30, 31, 32, 36];
const AI_REFERRER_WEIGHTS: [u32; 7] = [35, 25, 20, 15, 8, 5, 2];
/// UTM `source` values aligned with `AI_REFERRER_INDICES` (tracker source-param fallback).
const AI_SOURCE_PARAMS: [&str; 7] = [
    "chatgpt",
    "claude",
    "gemini",
    "grok",
    "perplexity",
    "copilot",
    "openai",
];
/// Share of AI-channel landings that include a `source` query param (referrer still set).
const AI_SOURCE_QUERY_SHARE: f64 = 0.32;

#[derive(Clone, Copy)]
enum LandingChannel {
    Search,
    Ai,
    Social,
    Campaign,
    Referrer,
    Direct,
}

#[derive(Default)]
struct FirstPagesDay {
    all: Vec<i64>,
    by_group: [Vec<i64>; SOURCE_GROUP_COUNT],
    /// Organic google.com subset of the Search group (for conversion bias).
    google_search: Vec<i64>,
    /// Non-Google organic search (bing / duckduckgo / brave).
    other_search: Vec<i64>,
}

impl FirstPagesDay {
    fn push(&mut self, visitor_timestamp_id: i64, source_group: SourceGroup, is_google: bool) {
        self.all.push(visitor_timestamp_id);
        self.by_group[source_group_index(source_group)].push(visitor_timestamp_id);
        if source_group == SourceGroup::Search {
            if is_google {
                self.google_search.push(visitor_timestamp_id);
            } else {
                self.other_search.push(visitor_timestamp_id);
            }
        }
    }
}

fn source_group_index(group: SourceGroup) -> usize {
    match group {
        SourceGroup::Search => 0,
        SourceGroup::Ai => 1,
        SourceGroup::Social => 2,
        SourceGroup::Campaigns => 3,
        SourceGroup::Referrers => 4,
        SourceGroup::Direct => 5,
    }
}

fn source_group_from_index(index: usize) -> SourceGroup {
    match index {
        0 => SourceGroup::Search,
        1 => SourceGroup::Ai,
        2 => SourceGroup::Social,
        3 => SourceGroup::Campaigns,
        4 => SourceGroup::Referrers,
        _ => SourceGroup::Direct,
    }
}

/// Assign each new conversion a source group so attributed *revenue* matches
/// `CONVERSION_GROUP_WEIGHTS`. Largest payments are placed first into the group
/// furthest below its target share (annual invoices otherwise dominate one bucket).
fn allocate_conversion_source_groups(amounts: &[i64], rng: &mut StdRng) -> Vec<SourceGroup> {
    debug_assert!(
        (CONVERSION_GROUP_WEIGHTS.iter().sum::<f64>() - 1.0).abs() < 1e-9,
        "conversion group weights must sum to 1.0"
    );
    let n = amounts.len();
    if n == 0 {
        return Vec::new();
    }
    let mut order: Vec<usize> = (0..n).collect();
    // Shuffle first so equal-amount ties are broken randomly, then place largest first.
    order.shuffle(rng);
    order.sort_by_key(|&idx| std::cmp::Reverse(amounts[idx]));
    let mut assigned_revenue = [0_i64; SOURCE_GROUP_COUNT];
    let mut groups = vec![SourceGroup::Direct; n];
    for &idx in &order {
        let amount = amounts[idx];
        let total_after = assigned_revenue.iter().sum::<i64>() + amount;
        let mut best_group = 0_usize;
        let mut best_score = f64::NEG_INFINITY;
        for group_idx in 0..SOURCE_GROUP_COUNT {
            let target = CONVERSION_GROUP_WEIGHTS[group_idx] * total_after as f64;
            let deficit = target - assigned_revenue[group_idx] as f64;
            // Tiny jitter breaks ties without overriding real deficits.
            let score = deficit + rng.gen_range(0.0..amount.max(1) as f64 * 1e-6);
            if score > best_score {
                best_score = score;
                best_group = group_idx;
            }
        }
        groups[idx] = source_group_from_index(best_group);
        assigned_revenue[best_group] += amount;
    }
    groups
}

fn pick_landing_channel(rng: &mut StdRng) -> LandingChannel {
    let roll = rng.gen_range(0.0..1.0);
    let mut threshold = LANDING_CHANNEL_SEARCH;
    if roll < threshold {
        return LandingChannel::Search;
    }
    threshold += LANDING_CHANNEL_AI;
    if roll < threshold {
        return LandingChannel::Ai;
    }
    threshold += LANDING_CHANNEL_SOCIAL;
    if roll < threshold {
        return LandingChannel::Social;
    }
    threshold += LANDING_CHANNEL_CAMPAIGN;
    if roll < threshold {
        return LandingChannel::Campaign;
    }
    threshold += LANDING_CHANNEL_REFERRER;
    if roll < threshold {
        return LandingChannel::Referrer;
    }
    debug_assert!(
        (LANDING_CHANNEL_SEARCH
            + LANDING_CHANNEL_AI
            + LANDING_CHANNEL_SOCIAL
            + LANDING_CHANNEL_CAMPAIGN
            + LANDING_CHANNEL_REFERRER
            + LANDING_CHANNEL_DIRECT
            - 1.0)
            .abs()
            < 1e-9
    );
    LandingChannel::Direct
}

struct ReferrerPickers {
    search: WeightedIndex<u32>,
    ai: WeightedIndex<u32>,
    google_ads: WeightedIndex<u32>,
    social_organic: WeightedIndex<u32>,
    campaign: WeightedIndex<u32>,
    other: WeightedIndex<u32>,
}

impl ReferrerPickers {
    fn new() -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            search: WeightedIndex::new(SEARCH_REFERRER_WEIGHTS)?,
            ai: WeightedIndex::new(AI_REFERRER_WEIGHTS)?,
            google_ads: WeightedIndex::new(GOOGLE_ADS_REFERRER_WEIGHTS)?,
            social_organic: WeightedIndex::new(SOCIAL_ORGANIC_REFERRER_WEIGHTS)?,
            campaign: WeightedIndex::new(CAMPAIGN_REFERRER_WEIGHTS)?,
            other: WeightedIndex::new(OTHER_REFERRER_WEIGHTS)?,
        })
    }

    fn pick_search(&self, referrer_ids: &[i64], rng: &mut StdRng) -> i64 {
        let idx = SEARCH_REFERRER_INDICES[self.search.sample(rng)];
        referrer_ids[idx]
    }

    fn pick_ai(&self, rng: &mut StdRng) -> (usize, usize) {
        let picker_idx = self.ai.sample(rng);
        (AI_REFERRER_INDICES[picker_idx], picker_idx)
    }

    fn pick_google_ads(&self, referrer_ids: &[i64], rng: &mut StdRng) -> i64 {
        let idx = GOOGLE_ADS_REFERRER_INDICES[self.google_ads.sample(rng)];
        referrer_ids[idx]
    }

    fn pick_social_organic(&self, referrer_ids: &[i64], rng: &mut StdRng) -> i64 {
        let idx = SOCIAL_ORGANIC_REFERRER_INDICES[self.social_organic.sample(rng)];
        referrer_ids[idx]
    }

    fn pick_campaign(&self, referrer_ids: &[i64], rng: &mut StdRng) -> i64 {
        let idx = CAMPAIGN_REFERRER_INDICES[self.campaign.sample(rng)];
        referrer_ids[idx]
    }

    fn pick_other(&self, referrer_ids: &[i64], rng: &mut StdRng) -> i64 {
        let idx = OTHER_REFERRER_INDICES[self.other.sample(rng)];
        referrer_ids[idx]
    }
}

fn pick_landing_attribution(
    channel: LandingChannel,
    referrer_ids: &[i64],
    referrer_pickers: &ReferrerPickers,
    rng: &mut StdRng,
) -> Result<(Option<i64>, Option<String>), Box<dyn Error>> {
    match channel {
        LandingChannel::Search => {
            if rng.gen_bool(SEARCH_GOOGLE_ADS_SHARE) {
                let referrer = Some(referrer_pickers.pick_google_ads(referrer_ids, rng));
                Ok((referrer, Some(google_ads_landing_query(rng)?)))
            } else {
                let referrer = Some(referrer_pickers.pick_search(referrer_ids, rng));
                Ok((referrer, None))
            }
        }
        LandingChannel::Ai => {
            let (referrer_idx, source_idx) = referrer_pickers.pick_ai(rng);
            let referrer = Some(referrer_ids[referrer_idx]);
            let landing_query = if rng.gen_bool(AI_SOURCE_QUERY_SHARE) {
                Some(ai_landing_query(AI_SOURCE_PARAMS[source_idx])?)
            } else {
                None
            };
            Ok((referrer, landing_query))
        }
        LandingChannel::Social => {
            if rng.gen_bool(SOCIAL_PAID_SHARE) {
                Ok(pick_paid_social_attribution(referrer_ids, rng))
            } else {
                let referrer = Some(referrer_pickers.pick_social_organic(referrer_ids, rng));
                Ok((referrer, None))
            }
        }
        LandingChannel::Campaign => {
            let referrer = if rng.gen_bool(0.88) {
                Some(referrer_pickers.pick_campaign(referrer_ids, rng))
            } else {
                None
            };
            Ok((referrer, Some(campaign_landing_query(rng)?)))
        }
        LandingChannel::Referrer => {
            let referrer = Some(referrer_pickers.pick_other(referrer_ids, rng));
            Ok((referrer, None))
        }
        LandingChannel::Direct => Ok((None, None)),
    }
}

struct ReferrerLandingPages {
    path_indices: Vec<usize>,
    picker: WeightedIndex<u32>,
}

struct LandingPathPickers {
    /// Pareto-weighted picker over all landing pages (search, AI, social, direct).
    all_pages: WeightedIndex<u32>,
    /// Per-referrer landing sets for the generic referrers bucket (1–5 pages each).
    by_referrer_idx: Vec<Option<ReferrerLandingPages>>,
}

impl LandingPathPickers {
    fn new(rng: &mut StdRng) -> Result<Self, Box<dyn Error>> {
        let all_pages = build_landing_path_picker(&LANDING_PATH_INDICES, rng)?;
        let mut by_referrer_idx: Vec<Option<ReferrerLandingPages>> =
            (0..REFERRER_COUNT).map(|_| None).collect();
        for &referrer_idx in &OTHER_REFERRER_INDICES {
            let page_count = rng.gen_range(1..=5);
            let path_indices = random_landing_path_subset(page_count, rng);
            let picker = build_landing_path_picker(&path_indices, rng)?;
            by_referrer_idx[referrer_idx] = Some(ReferrerLandingPages {
                path_indices,
                picker,
            });
        }
        Ok(Self {
            all_pages,
            by_referrer_idx,
        })
    }

    fn pick(
        &self,
        source_group: SourceGroup,
        referrer_id: Option<i64>,
        referrer_ids: &[i64],
        path_ids: &[i64],
        rng: &mut StdRng,
    ) -> (usize, i64) {
        if source_group == SourceGroup::Campaigns {
            return (PATH_HOME, path_ids[PATH_HOME]);
        }
        if source_group == SourceGroup::Referrers
            && let Some(id) = referrer_id
            && let Some(referrer_idx) = referrer_ids.iter().position(|&row_id| row_id == id)
            && let Some(pages) = self
                .by_referrer_idx
                .get(referrer_idx)
                .and_then(|entry| entry.as_ref())
        {
            let local_idx = pages.picker.sample(rng);
            let path_idx = pages.path_indices[local_idx];
            return (path_idx, path_ids[path_idx]);
        }
        let idx = self.all_pages.sample(rng);
        let path_idx = LANDING_PATH_INDICES[idx];
        (path_idx, path_ids[path_idx])
    }
}

fn random_landing_path_subset(count: usize, rng: &mut StdRng) -> Vec<usize> {
    let mut indices: Vec<usize> = LANDING_PATH_INDICES.to_vec();
    indices.shuffle(rng);
    indices.truncate(count);
    indices
}

fn build_landing_path_picker(
    path_indices: &[usize],
    rng: &mut StdRng,
) -> Result<WeightedIndex<u32>, Box<dyn Error>> {
    if path_indices.is_empty() {
        return Err("landing path picker requires at least one path".into());
    }
    if path_indices.len() == 1 {
        return Ok(WeightedIndex::new([1_u32])?);
    }
    let weights = sample_pareto_weights(path_indices.len(), LANDING_PARETO_ALPHA, rng);
    weighted_index_from_f64(&weights)
}

fn landing_query_will_create_campaign(landing_query: Option<&str>) -> bool {
    let Some(query_json) = landing_query else {
        return false;
    };
    let Some((maybe_tuple, _)) = revscope::prepare_landing_attribution_from_json(query_json) else {
        return false;
    };
    maybe_tuple
        .as_ref()
        .is_some_and(|tuple| tuple.has_campaign())
}

fn pick_next_path(from_path_idx: usize, path_ids: &[i64], rng: &mut StdRng) -> (usize, i64) {
    let choices = next_path_indices(from_path_idx);
    let next_idx = *choices
        .choose(rng)
        .expect("navigation graph should always have next paths");
    (next_idx, path_ids[next_idx])
}

fn ensure_campaign_id(
    conn: &Connection,
    tuple: &revscope::LandingCampaignTuple,
    first_seen_at: i64,
) -> Result<i64, Box<dyn Error>> {
    debug_assert!(tuple.has_campaign());
    conn.execute(
        &format!(
            "INSERT INTO campaigns (source, medium, campaign, network, first_seen_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             {CAMPAIGNS_INSERT_ON_CONFLICT}"
        ),
        params![
            &tuple.source,
            &tuple.medium,
            &tuple.campaign,
            tuple.network.as_deref(),
            first_seen_at,
        ],
    )?;
    conn.query_row(
        "SELECT id FROM campaigns
         WHERE source = ?1
           AND medium = ?2
           AND campaign = ?3
           AND COALESCE(network, '') = COALESCE(?4, '')",
        params![
            &tuple.source,
            &tuple.medium,
            &tuple.campaign,
            tuple.network.as_deref(),
        ],
        |row| row.get(0),
    )
    .map_err(|error| -> Box<dyn Error> {
        if matches!(error, rusqlite::Error::QueryReturnedNoRows) {
            "missing campaigns row after insert".into()
        } else {
            error.into()
        }
    })
}

/// Splits a landing metadata JSON object into a campaigns row and remaining pageview metadata.
fn split_landing_attribution(
    conn: &Connection,
    query_json: &str,
    first_seen_at: i64,
) -> Result<(Option<i64>, Option<String>, Option<String>), Box<dyn Error>> {
    let Some((maybe_tuple, remaining)) =
        revscope::prepare_landing_attribution_from_json(query_json)
    else {
        return Err("landing query must be a JSON object".into());
    };
    let click_id = maybe_tuple
        .as_ref()
        .and_then(|tuple| tuple.click_id.clone());
    let campaign_id = match maybe_tuple {
        Some(tuple) => Some(ensure_campaign_id(conn, &tuple, first_seen_at)?),
        None => None,
    };
    Ok((campaign_id, remaining, click_id))
}

fn pick_paid_social_attribution(
    referrer_ids: &[i64],
    rng: &mut StdRng,
) -> (Option<i64>, Option<String>) {
    let roll = rng.gen_range(0..4);
    if roll == 0 {
        let idx =
            PAID_SOCIAL_LINKEDIN_INDICES[rng.gen_range(0..PAID_SOCIAL_LINKEDIN_INDICES.len())];
        (
            Some(referrer_ids[idx]),
            Some(paid_social_landing_query(rng, "linkedin").expect("paid social query json")),
        )
    } else if roll == 1 {
        let idx = PAID_SOCIAL_X_INDICES[rng.gen_range(0..PAID_SOCIAL_X_INDICES.len())];
        (
            Some(referrer_ids[idx]),
            Some(paid_social_landing_query(rng, "x").expect("paid social query json")),
        )
    } else if roll == 2 {
        let idx = META_ADS_REFERRER_INDICES[0];
        (
            Some(referrer_ids[idx]),
            Some(meta_ads_landing_query(rng, "facebook").expect("meta ads query json")),
        )
    } else {
        let idx = META_ADS_REFERRER_INDICES[1];
        (
            Some(referrer_ids[idx]),
            Some(meta_ads_landing_query(rng, "instagram").expect("meta ads query json")),
        )
    }
}

/// AI assistant landings via `source` query param (mirrors tracker / ingest fallback).
fn ai_landing_query(source: &str) -> Result<String, serde_json::Error> {
    Ok(json!({ "source": source }).to_string())
}

fn random_click_id_value(rng: &mut StdRng) -> String {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-";
    let len = rng.gen_range(28..=96);
    (0..len)
        .map(|_| char::from(CHARSET[rng.gen_range(0..CHARSET.len())]))
        .collect()
}

fn google_click_id_param(rng: &mut StdRng) -> (&'static str, String) {
    let roll = rng.gen_range(0..100);
    let key = if roll < 7 {
        "gbraid"
    } else if roll < 12 {
        "wbraid"
    } else {
        "gclid"
    };
    (key, random_click_id_value(rng))
}

fn paid_social_click_id_param(source: &str, rng: &mut StdRng) -> (&'static str, String) {
    let key = match source {
        "linkedin" => "li_fat_id",
        "x" => "twclid",
        _ => "click_id",
    };
    (key, random_click_id_value(rng))
}

/// Google Search Ads landings: UTM-style campaign tags (classified as google_ads/campaign).
fn google_ads_landing_query(rng: &mut StdRng) -> Result<String, serde_json::Error> {
    let campaign = [
        "brand-search",
        "competitor-search",
        "retargeting-search",
        "midmarket-search",
    ];
    let (click_key, click_value) = google_click_id_param(rng);
    Ok(json!({
        "source": "google",
        "medium": "cpc",
        "campaign": campaign.choose(rng).expect("campaign list should not be empty"),
        (click_key): click_value,
        "adgroup": if rng.gen_bool(0.45) { Some("saas-attribution") } else { None },
        "term": if rng.gen_bool(0.35) { Some("revenue attribution software") } else { None },
    })
    .to_string())
}

/// Paid social on LinkedIn / X (same host as organic posts; params carry the campaign).
fn paid_social_landing_query(rng: &mut StdRng, source: &str) -> Result<String, serde_json::Error> {
    let campaign = ["brand", "midmarket", "retargeting", "partner"];
    let (click_key, click_value) = paid_social_click_id_param(source, rng);
    Ok(json!({
        "source": source,
        "medium": "paid-social",
        "campaign": campaign.choose(rng).expect("campaign list should not be empty"),
        (click_key): click_value,
        "content": if rng.gen_bool(0.4) {
            Some(format!("creative-{}", rng.gen_range(1..=6)))
        } else {
            None
        },
    })
    .to_string())
}

/// Meta Ads landings: paid-social UTMs (classified as meta_ads/campaign).
fn meta_ads_landing_query(rng: &mut StdRng, source: &str) -> Result<String, serde_json::Error> {
    let campaign = ["brand", "midmarket", "retargeting", "prospecting"];
    Ok(json!({
        "source": source,
        "medium": "paid-social",
        "campaign": campaign.choose(rng).expect("campaign list should not be empty"),
        "fbclid": random_click_id_value(rng),
        "content": if rng.gen_bool(0.5) {
            Some(format!("adset-{}", rng.gen_range(1..=5)))
        } else {
            None
        },
    })
    .to_string())
}

/// Query params that classify as paid/ads/email/affiliate (dashboard `campaigns` bucket).
fn campaign_landing_query(rng: &mut StdRng) -> Result<String, serde_json::Error> {
    let campaign = ["brand", "midmarket", "retargeting", "partner", "podcast"];
    let payload = match rng.gen_range(0..3) {
        0 => {
            let (click_key, click_value) = google_click_id_param(rng);
            json!({
                "source": "google",
                "medium": "cpc",
                "campaign": campaign.choose(rng).expect("campaign list should not be empty"),
                (click_key): click_value,
                "adgroup": if rng.gen_bool(0.4) { Some("saas-attribution") } else { None },
            })
        }
        1 => json!({
            "source": "newsletter",
            "medium": "email",
            "campaign": campaign.choose(rng).expect("campaign list should not be empty"),
        }),
        _ => json!({
            "medium": "affiliate",
            "campaign": campaign.choose(rng).expect("campaign list should not be empty"),
            "aff_click_id": random_click_id_value(rng),
        }),
    };
    Ok(payload.to_string())
}

fn maybe_landing_metadata(rng: &mut StdRng) -> Result<Option<String>, serde_json::Error> {
    if !rng.gen_bool(0.55) {
        return Ok(None);
    }
    Ok(Some(
        json!({
            "ab_variant": if rng.gen_bool(0.5) { "pricing-a" } else { "pricing-b" },
            "session_quality": rng.gen_range(35..=100),
            "entrypoint": if rng.gen_bool(0.6) { "marketing-site" } else { "docs" }
        })
        .to_string(),
    ))
}

fn maybe_page_metadata(rng: &mut StdRng) -> Result<Option<String>, serde_json::Error> {
    if !rng.gen_bool(0.25) {
        return Ok(None);
    }
    Ok(Some(
        json!({
            "component": if rng.gen_bool(0.5) { "feature-grid" } else { "pricing-table" },
            "experiment": if rng.gen_bool(0.4) { "copy-test-2" } else { "none" }
        })
        .to_string(),
    ))
}

#[allow(clippy::too_many_arguments)]
fn insert_pageview(
    conn: &Connection,
    timestamp_id: i64,
    visitor_timestamp_id: i64,
    previous_timestamp_id: Option<i64>,
    path_id: i64,
    metadata_json: Option<&str>,
    duration_seconds: i64,
    scrolled_percent: i64,
) -> Result<(), Box<dyn Error>> {
    conn.execute(
        "INSERT INTO pageviews (
            timestamp_id, visitor_timestamp_id, previous_timestamp_id, path_id,
            duration_seconds, scrolled_percent
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            timestamp_id,
            visitor_timestamp_id,
            previous_timestamp_id,
            path_id,
            duration_seconds,
            scrolled_percent,
        ],
    )?;
    upsert_metadata_rows(conn, visitor_timestamp_id, timestamp_id, metadata_json)?;
    Ok(())
}

fn upsert_metadata_rows(
    conn: &Connection,
    visitor_timestamp_id: i64,
    page_timestamp_id: i64,
    metadata_json: Option<&str>,
) -> Result<(), Box<dyn Error>> {
    let Some(metadata_json) = metadata_json else {
        return Ok(());
    };
    let Value::Object(metadata) = serde_json::from_str::<Value>(metadata_json)? else {
        return Ok(());
    };
    for (key, value) in metadata {
        conn.execute(
            "INSERT INTO metadata (visitor_timestamp_id, key, value, page_timestamp_id)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(visitor_timestamp_id, key) DO UPDATE SET
                value = excluded.value,
                page_timestamp_id = excluded.page_timestamp_id",
            params![
                visitor_timestamp_id,
                key,
                metadata_value_text(&value),
                page_timestamp_id
            ],
        )?;
    }
    Ok(())
}

fn metadata_value_text(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Array(_) | Value::Object(_) => serde_json::to_string(value).unwrap_or_default(),
        Value::Null => "null".to_string(),
    }
}

fn event_metadata(rng: &mut StdRng) -> Result<String, serde_json::Error> {
    Ok(json!({
        "source_component": if rng.gen_bool(0.45) { "hero-cta" } else { "sidebar" },
        "value": rng.gen_range(1..=5),
        "label": if rng.gen_bool(0.4) { "high-intent" } else { "warm" }
    })
    .to_string())
}

/// External hosts for synthetic revscope-outbound-click events (weight = relative frequency).
const OUTBOUND_CLICK_DESTINATIONS: [(&str, u32); 7] = [
    ("youtube.com", 32),
    ("x.com", 20),
    ("linkedin.com", 20),
    ("cnil.fr", 10),
    ("ec.europa.eu", 10),
    ("gdpr.eu", 7),
    ("iabeurope.eu", 5),
];

fn pick_outbound_click_destination(rng: &mut StdRng) -> &'static str {
    let total = OUTBOUND_CLICK_DESTINATIONS
        .iter()
        .map(|(_, weight)| *weight)
        .sum();
    let mut roll = rng.gen_range(0..total);
    for (host, weight) in OUTBOUND_CLICK_DESTINATIONS {
        if roll < weight {
            return host;
        }
        roll -= weight;
    }
    OUTBOUND_CLICK_DESTINATIONS[0].0
}

fn outbound_click_metadata(to: &str) -> String {
    json!({ "to": to }).to_string()
}

fn path_eligible_for_outbound_click(path_idx: usize) -> bool {
    matches!(
        path_idx,
        PATH_HOME
            | PATH_FEATURES
            | PATH_PRICING
            | PATH_BLOG
            | PATH_BLOG_SAAS
            | PATH_BLOG_CHURN
            | PATH_DOCS
            | PATH_DOCS_START
    )
}

fn maybe_emit_outbound_click(
    conn: &Connection,
    event_name_ids: &[i64],
    page_timestamp_id: i64,
    page_duration_seconds: i64,
    generated_timestamp_ids: &mut HashSet<i64>,
    rng: &mut StdRng,
) -> Result<(), Box<dyn Error>> {
    if !rng.gen_bool(OUTBOUND_CLICK_PROB) {
        return Ok(());
    }
    let Some(event_timestamp_id) = make_event_timestamp_id(
        page_timestamp_id,
        page_duration_seconds.max(2),
        None,
        generated_timestamp_ids,
        rng,
    ) else {
        return Ok(());
    };
    conn.execute(
        "INSERT INTO events (timestamp_id, event_name_id, page_timestamp_id, metadata)
         VALUES (?1, ?2, ?3, jsonb(?4))",
        params![
            event_timestamp_id,
            event_name_ids[OUTBOUND_CLICK_EVENT_INDEX],
            page_timestamp_id,
            outbound_click_metadata(pick_outbound_click_destination(rng)),
        ],
    )?;
    Ok(())
}

fn maybe_emit_outbound_click_for_path(
    conn: &Connection,
    event_name_ids: &[i64],
    path_idx: usize,
    page_timestamp_id: i64,
    page_duration_seconds: i64,
    generated_timestamp_ids: &mut HashSet<i64>,
    rng: &mut StdRng,
) -> Result<(), Box<dyn Error>> {
    if !path_eligible_for_outbound_click(path_idx) {
        return Ok(());
    }
    maybe_emit_outbound_click(
        conn,
        event_name_ids,
        page_timestamp_id,
        page_duration_seconds,
        generated_timestamp_ids,
        rng,
    )
}

fn plan_tier_for_amount(monthly_amount: i64) -> &'static str {
    match monthly_amount {
        0..=12_000 => "starter",
        12_001..=35_000 => "growth",
        35_001..=95_000 => "scale",
        _ => "enterprise",
    }
}

fn subscription_period_seconds(interval_unit: &str) -> i64 {
    match interval_unit {
        "year" => YEAR_SECONDS,
        _ => MONTH_SECONDS,
    }
}

/// The recurring price normalized to a month, matching how the backend derives
/// MRR from a plan amount.
fn subscription_monthly_amount(amount: i64, interval_unit: &str) -> i64 {
    match interval_unit {
        "year" => amount / 12,
        _ => amount,
    }
}

fn subscription_billing_cycle(interval_unit: &str) -> &'static str {
    if interval_unit == "year" {
        "annual"
    } else {
        "monthly"
    }
}

/// Picks monthly vs annual for a new subscription; annual payment is 12× the monthly plan amount.
fn subscription_plan_for_new(
    remaining_budget: i64,
    monthly_amount: i64,
    annual_share: f64,
    rng: &mut StdRng,
) -> (&'static str, i64) {
    let wants_annual = rng.gen_bool(annual_share);
    let annual_amount = monthly_amount.saturating_mul(12);
    if wants_annual && annual_amount <= remaining_budget.saturating_add(8_000) {
        ("year", annual_amount)
    } else {
        ("month", monthly_amount)
    }
}

/// Daily revenue shape with up to 5× spread between the quietest and busiest days.
fn generate_day_revenue_weights(rng: &mut StdRng) -> Vec<f64> {
    let mut weights = vec![0.0_f64; DAYS_PER_MONTH];
    for weight in &mut weights {
        *weight = rng.gen_range(DAY_REVENUE_WEIGHT_MIN..=DAY_REVENUE_WEIGHT_MAX);
    }
    let low_day = rng.gen_range(0..DAYS_PER_MONTH);
    let mut high_day = rng.gen_range(0..DAYS_PER_MONTH);
    while high_day == low_day {
        high_day = rng.gen_range(0..DAYS_PER_MONTH);
    }
    weights[low_day] = DAY_REVENUE_WEIGHT_MIN;
    weights[high_day] = DAY_REVENUE_WEIGHT_MAX;
    weights
}

fn allocate_i64_by_weights(total: i64, weights: &[f64]) -> Vec<i64> {
    if total <= 0 {
        return vec![0; weights.len()];
    }
    let weight_sum: f64 = weights.iter().sum();
    let mut amounts: Vec<i64> = weights
        .iter()
        .map(|w| ((total as f64) * w / weight_sum).floor() as i64)
        .collect();
    let remainder = (total - amounts.iter().sum::<i64>()).max(0) as usize;
    let mut fractional: Vec<(usize, f64)> = weights
        .iter()
        .enumerate()
        .map(|(day_idx, w)| {
            let exact = (total as f64) * w / weight_sum;
            (day_idx, exact - amounts[day_idx] as f64)
        })
        .collect();
    fractional.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    for (day_idx, _) in fractional.into_iter().take(remainder) {
        amounts[day_idx] += 1;
    }
    amounts
}

fn allocate_usize_by_weights(total: usize, weights: &[f64]) -> Vec<usize> {
    if total == 0 {
        return vec![0; weights.len()];
    }
    let weight_sum: f64 = weights.iter().sum();
    let mut counts: Vec<usize> = weights
        .iter()
        .map(|w| ((total as f64) * w / weight_sum).floor() as usize)
        .collect();
    let remainder = total.saturating_sub(counts.iter().sum());
    let mut fractional: Vec<(usize, f64)> = weights
        .iter()
        .enumerate()
        .map(|(day_idx, w)| {
            let exact = (total as f64) * w / weight_sum;
            (day_idx, exact - counts[day_idx] as f64)
        })
        .collect();
    fractional.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    for (day_idx, _) in fractional.into_iter().take(remainder) {
        counts[day_idx] += 1;
    }
    counts
}

/// Visitor counts follow planned daily conversions, with a small baseline for non-converting traffic.
fn allocate_visitors_by_daily_conversions(
    total: usize,
    day_conversions: &[usize],
    baseline_weights: &[f64],
    rng: &mut StdRng,
) -> Vec<usize> {
    let conversion_sum: usize = day_conversions.iter().sum();
    if conversion_sum == 0 {
        return allocate_usize_by_weights(total, baseline_weights);
    }

    let average_daily_conversions = conversion_sum as f64 / day_conversions.len().max(1) as f64;
    let baseline_weight_sum: f64 = baseline_weights.iter().sum();
    let average_baseline_weight = baseline_weight_sum / baseline_weights.len().max(1) as f64;
    let baseline_conversions = average_daily_conversions * VISITOR_BASELINE_CONVERSION_SHARE;

    let weights: Vec<f64> = day_conversions
        .iter()
        .zip(baseline_weights.iter())
        .map(|(&conversions, baseline_weight)| {
            let normalized_baseline = if average_baseline_weight > 0.0 {
                baseline_weight / average_baseline_weight
            } else {
                1.0
            };
            let correlated = conversions as f64 + baseline_conversions * normalized_baseline;
            correlated * rng.gen_range(VISITOR_DAY_NOISE_MIN..=VISITOR_DAY_NOISE_MAX)
        })
        .collect();
    allocate_usize_by_weights(total, &weights)
}

fn choose_first_page_for_day(
    day_first_pages: &[FirstPagesDay],
    fallback_first_pages: &[i64],
    day_idx: usize,
    rng: &mut StdRng,
) -> i64 {
    *day_first_pages
        .get(day_idx)
        .and_then(|day| day.all.choose(rng))
        .or_else(|| fallback_first_pages.choose(rng))
        .expect("month should include sessions")
}

fn session_start_seconds(visitor_timestamp_id: i64) -> i64 {
    visitor_timestamp_id / NANOS_PER_SECOND
}

fn visitor_eligible_for_payment(visitor_timestamp_id: i64, paid_at: i64) -> bool {
    paid_at - session_start_seconds(visitor_timestamp_id) >= MIN_TIME_TO_PURCHASE_SECONDS
}

fn choose_eligible_visitor(ids: &[i64], paid_at: i64, rng: &mut StdRng) -> Option<i64> {
    let eligible: Vec<i64> = ids
        .iter()
        .copied()
        .filter(|id| visitor_eligible_for_payment(*id, paid_at))
        .collect();
    eligible.choose(rng).copied()
}

fn choose_search_conversion_visitor(
    day: &FirstPagesDay,
    paid_at: i64,
    rng: &mut StdRng,
) -> Option<i64> {
    // Attribute Search conversions to google.com whenever possible. Non-Google
    // engines still appear in session volume (~5%) via SEARCH_REFERRER_WEIGHTS;
    // giving them converting annual invoices makes Search revenue look 50/50.
    choose_eligible_visitor(&day.google_search, paid_at, rng)
        .or_else(|| choose_eligible_visitor(&day.other_search, paid_at, rng))
}

fn choose_first_page_from_month_day(
    month_first_pages_by_day: &[FirstPagesDay],
    day_idx: usize,
    preferred_group: Option<SourceGroup>,
    paid_at: i64,
    rng: &mut StdRng,
) -> Option<i64> {
    let day = month_first_pages_by_day.get(day_idx)?;
    if let Some(group) = preferred_group {
        if group == SourceGroup::Search {
            return choose_search_conversion_visitor(day, paid_at, rng);
        }
        // Stay on the preferred channel; callers walk earlier days in-month first.
        return choose_eligible_visitor(&day.by_group[source_group_index(group)], paid_at, rng);
    }
    choose_eligible_visitor(&day.all, paid_at, rng)
}

/// Attribute a conversion to a first-visit in the *same month* as the payment.
/// Revenue-sources only counts sessions that began in the selected range, so
/// cross-month lookback would hide the balanced channel mix from the demo UI.
fn choose_first_page_for_conversion(
    month_first_pages_by_day: &[FirstPagesDay],
    paid_day: usize,
    paid_at: i64,
    preferred_group: SourceGroup,
    rng: &mut StdRng,
) -> i64 {
    let preferred = Some(preferred_group);
    // Prefer same-day, then walk backward within the month only.
    for day_idx in (0..=paid_day).rev() {
        if let Some(visitor_timestamp_id) = choose_first_page_from_month_day(
            month_first_pages_by_day,
            day_idx,
            preferred,
            paid_at,
            rng,
        ) {
            return visitor_timestamp_id;
        }
    }

    for day_idx in (0..=paid_day).rev() {
        if let Some(visitor_timestamp_id) =
            choose_first_page_from_month_day(month_first_pages_by_day, day_idx, None, paid_at, rng)
        {
            return visitor_timestamp_id;
        }
    }

    month_first_pages_by_day
        .iter()
        .flat_map(|day| day.all.iter())
        .copied()
        .find(|visitor_timestamp_id| visitor_eligible_for_payment(*visitor_timestamp_id, paid_at))
        .expect("month should include a landing session before payment")
}

fn target_conversion_session_duration(time_to_purchase: i64, rng: &mut StdRng) -> i64 {
    if time_to_purchase >= 30 * DAY_SECONDS {
        rng.gen_range(1_800..=7_200)
    } else if time_to_purchase >= 7 * DAY_SECONDS {
        rng.gen_range(900..=3_600)
    } else if time_to_purchase >= DAY_SECONDS {
        rng.gen_range(600..=2_400)
    } else {
        rng.gen_range(MIN_CONVERSION_SESSION_DURATION_SECONDS..=1_800)
    }
}

fn extend_conversion_session(
    conn: &Connection,
    visitor_timestamp_id: i64,
    paid_at: i64,
    rng: &mut StdRng,
) -> Result<(), Box<dyn Error>> {
    let time_to_purchase = paid_at - session_start_seconds(visitor_timestamp_id);
    let target_duration = target_conversion_session_duration(time_to_purchase, rng)
        .max(MIN_CONVERSION_SESSION_DURATION_SECONDS);
    let current_duration: i64 = conn.query_row(
        "SELECT session_duration_seconds
         FROM landings
         WHERE visitor_timestamp_id = ?1",
        params![visitor_timestamp_id],
        |row| row.get(0),
    )?;
    if target_duration <= current_duration {
        return Ok(());
    }

    let deficit = target_duration - current_duration;
    conn.execute(
        "UPDATE pageviews
         SET duration_seconds = duration_seconds + ?2
         WHERE timestamp_id = ?1",
        params![visitor_timestamp_id, deficit],
    )?;
    conn.execute(
        "UPDATE landings
         SET session_duration_seconds = ?2
         WHERE visitor_timestamp_id = ?1",
        params![visitor_timestamp_id, target_duration],
    )?;
    Ok(())
}

fn home_funnel_event_path_idx(event_step: usize) -> usize {
    match event_step {
        0..=3 => PATH_HOME,
        4 | 5 => PATH_SIGNUP,
        6 => PATH_PRICING,
        _ => PATH_CHECKOUT,
    }
}

fn insert_funnel_event(
    conn: &Connection,
    event_name_ids: &[i64],
    event_step: usize,
    page_timestamp_id: i64,
    page_duration_seconds: i64,
    max_timestamp_id: Option<i64>,
    generated_timestamp_ids: &mut HashSet<i64>,
    rng: &mut StdRng,
) -> Result<(), Box<dyn Error>> {
    let Some(event_timestamp_id) = make_event_timestamp_id(
        page_timestamp_id,
        page_duration_seconds.max(2),
        max_timestamp_id,
        generated_timestamp_ids,
        rng,
    ) else {
        return Ok(());
    };
    conn.execute(
        "INSERT INTO events (timestamp_id, event_name_id, page_timestamp_id, metadata)
         VALUES (?1, ?2, ?3, jsonb(?4))",
        params![
            event_timestamp_id,
            event_name_ids[HOME_FUNNEL_EVENT_INDICES[event_step]],
            page_timestamp_id,
            event_metadata(rng)?,
        ],
    )?;
    Ok(())
}

fn insert_sequential_funnel_event(
    conn: &Connection,
    event_name_ids: &[i64],
    event_step: usize,
    page_timestamp_id: i64,
    page_duration_seconds: i64,
    max_timestamp_id: Option<i64>,
    cursor: &mut i64,
    generated_timestamp_ids: &mut HashSet<i64>,
    rng: &mut StdRng,
) -> Result<(), Box<dyn Error>> {
    let page_end = page_timestamp_id + page_duration_seconds.max(2) * NANOS_PER_SECOND - 1;
    let upper_bound = max_timestamp_id.unwrap_or(page_end).min(page_end);
    let page_start = page_timestamp_id + NANOS_PER_SECOND + 1;
    if *cursor < page_start {
        *cursor = page_start;
    }
    if *cursor > upper_bound {
        return Ok(());
    }
    let step_nanos = rng.gen_range(NANOS_PER_SECOND..=3 * NANOS_PER_SECOND);
    *cursor = (*cursor + step_nanos).min(upper_bound);
    while !generated_timestamp_ids.insert(*cursor) {
        *cursor += 1;
        if *cursor > upper_bound {
            return Ok(());
        }
    }
    conn.execute(
        "INSERT INTO events (timestamp_id, event_name_id, page_timestamp_id, metadata)
         VALUES (?1, ?2, ?3, jsonb(?4))",
        params![
            *cursor,
            event_name_ids[HOME_FUNNEL_EVENT_INDICES[event_step]],
            page_timestamp_id,
            event_metadata(rng)?,
        ],
    )?;
    Ok(())
}

fn emit_home_funnel_events(
    conn: &Connection,
    event_name_ids: &[i64],
    home_page_timestamp_id: i64,
    home_duration_seconds: i64,
    complete_all: bool,
    generated_timestamp_ids: &mut HashSet<i64>,
    rng: &mut StdRng,
) -> Result<(), Box<dyn Error>> {
    let mut page_duration = home_duration_seconds.max(10);
    for event_step in 0..HOME_FUNNEL_EVENT_INDICES.len() {
        if !complete_all && !rng.gen_bool(HOME_FUNNEL_STEP_CONTINUE[event_step]) {
            break;
        }
        insert_funnel_event(
            conn,
            event_name_ids,
            event_step,
            home_page_timestamp_id,
            page_duration,
            None,
            generated_timestamp_ids,
            rng,
        )?;
        page_duration += rng.gen_range(2..=8);
    }
    Ok(())
}

fn emit_home_funnel_events_on_pages(
    conn: &Connection,
    event_name_ids: &[i64],
    pages: &[(usize, i64, i64)],
    events_through: usize,
    max_timestamp_id: Option<i64>,
    generated_timestamp_ids: &mut HashSet<i64>,
    rng: &mut StdRng,
) -> Result<(), Box<dyn Error>> {
    let last_event_step = events_through.min(HOME_FUNNEL_EVENT_INDICES.len() - 1);
    let mut event_cursor = 0_i64;
    for event_step in 0..=last_event_step {
        let target_path = home_funnel_event_path_idx(event_step);
        let Some(&(_, page_timestamp_id, page_duration_seconds)) = pages
            .iter()
            .find(|&&(path_idx, _, _)| path_idx == target_path)
        else {
            continue;
        };
        insert_sequential_funnel_event(
            conn,
            event_name_ids,
            event_step,
            page_timestamp_id,
            page_duration_seconds,
            max_timestamp_id,
            &mut event_cursor,
            generated_timestamp_ids,
            rng,
        )?;
    }
    Ok(())
}

const HOME_CONVERSION_PATH_CHAIN: [usize; 4] =
    [PATH_HOME, PATH_SIGNUP, PATH_PRICING, PATH_CHECKOUT];
/// Minimum seconds between the last funnel event and payment / abandon time.
const MIN_SECONDS_BEFORE_PAYMENT: i64 = 5;
/// Conversion checkout pages use shorter dwell times so events finish before payment.
const CONVERSION_PAGE_DURATION_RANGE: std::ops::RangeInclusive<i64> = 15..=45;

fn home_funnel_path_chain_through(events_through: usize) -> &'static [usize] {
    match events_through {
        0..=3 => &HOME_CONVERSION_PATH_CHAIN[..1],
        4..=5 => &HOME_CONVERSION_PATH_CHAIN[..2],
        6 => &HOME_CONVERSION_PATH_CHAIN[..3],
        _ => &HOME_CONVERSION_PATH_CHAIN,
    }
}

fn insert_checkout_before_purchase(
    conn: &Connection,
    path_ids: &[i64],
    event_name_ids: &[i64],
    visitor_timestamp_id: i64,
    paid_at: i64,
    generated_timestamp_ids: &mut HashSet<i64>,
    rng: &mut StdRng,
) -> Result<(), Box<dyn Error>> {
    insert_home_funnel_checkout_journey(
        conn,
        path_ids,
        event_name_ids,
        visitor_timestamp_id,
        paid_at,
        HOME_FUNNEL_EVENT_INDICES.len() - 1,
        generated_timestamp_ids,
        rng,
    )
}

fn insert_home_funnel_checkout_journey(
    conn: &Connection,
    path_ids: &[i64],
    event_name_ids: &[i64],
    visitor_timestamp_id: i64,
    journey_end_at: i64,
    events_through: usize,
    generated_timestamp_ids: &mut HashSet<i64>,
    rng: &mut StdRng,
) -> Result<(), Box<dyn Error>> {
    let journey_end_nanos = journey_end_at.saturating_mul(NANOS_PER_SECOND);
    let max_event_timestamp_id = journey_end_nanos - MIN_SECONDS_BEFORE_PAYMENT * NANOS_PER_SECOND;
    let previous_timestamp_id: i64 = conn
        .query_row(
            "SELECT timestamp_id FROM pageviews
             WHERE visitor_timestamp_id = ?1
               AND timestamp_id < ?2
             ORDER BY timestamp_id DESC
             LIMIT 1",
            params![visitor_timestamp_id, journey_end_nanos],
            |row| row.get(0),
        )
        .unwrap_or(visitor_timestamp_id);
    let last_page_at = previous_timestamp_id / NANOS_PER_SECOND;

    let path_chain = home_funnel_path_chain_through(events_through);
    let last_page_start = journey_end_at - rng.gen_range(45..=120);
    let mut page_times = vec![0_i64; path_chain.len()];
    page_times[path_chain.len() - 1] = last_page_start;
    for idx in (0..path_chain.len() - 1).rev() {
        page_times[idx] = page_times[idx + 1] - rng.gen_range(20..=90);
    }

    let min_start = last_page_at + 60;
    if page_times[0] < min_start {
        let shift = min_start - page_times[0];
        for page_time in &mut page_times {
            *page_time += shift;
        }
    }
    if page_times[path_chain.len() - 1] >= journey_end_at - MIN_SECONDS_BEFORE_PAYMENT {
        let shift =
            page_times[path_chain.len() - 1] - (journey_end_at - MIN_SECONDS_BEFORE_PAYMENT) + 1;
        for page_time in &mut page_times {
            *page_time -= shift;
        }
    }

    let mut previous_id = previous_timestamp_id;
    let mut funnel_pages = Vec::with_capacity(path_chain.len());
    for (page_idx, (&path_idx, &page_time)) in path_chain.iter().zip(page_times.iter()).enumerate()
    {
        let page_timestamp_id = make_unique_timestamp_id(page_time, generated_timestamp_ids, rng);
        apply_pageview_duration_cap(conn, previous_id, page_timestamp_id, &mut 0_i64, &mut [])?;
        let client_page_id = make_96bit_id(rng);
        let is_last_page = page_idx == path_chain.len() - 1;
        let max_duration_seconds = if is_last_page {
            (journey_end_at - page_time - MIN_SECONDS_BEFORE_PAYMENT).max(5)
        } else {
            i64::MAX
        };
        let page_duration_seconds = rng
            .gen_range(CONVERSION_PAGE_DURATION_RANGE)
            .min(max_duration_seconds);
        let page_scrolled_percent = rng.gen_range(30..=100);

        let page_metadata = maybe_page_metadata(rng)?;
        insert_pageview(
            conn,
            page_timestamp_id,
            visitor_timestamp_id,
            Some(previous_id),
            path_ids[path_idx],
            page_metadata.as_deref(),
            page_duration_seconds,
            page_scrolled_percent,
        )?;
        conn.execute(
            "INSERT INTO client_page_ids_map (visitor_timestamp_id, page_id)
             VALUES (?1, ?2)",
            params![page_timestamp_id, client_page_id],
        )?;
        funnel_pages.push((path_idx, page_timestamp_id, page_duration_seconds));
        previous_id = page_timestamp_id;
    }

    clear_home_funnel_events(conn, event_name_ids, visitor_timestamp_id)?;
    emit_home_funnel_events_on_pages(
        conn,
        event_name_ids,
        &funnel_pages,
        events_through,
        Some(max_event_timestamp_id),
        generated_timestamp_ids,
        rng,
    )?;

    sync_landing_session_metrics(conn, visitor_timestamp_id)?;

    Ok(())
}

fn pick_unconverted_home_landing(
    conn: &Connection,
    home_path_id: i64,
    min_visitor_timestamp_id: i64,
    max_last_page_at: i64,
    exclude: &HashSet<i64>,
    rng: &mut StdRng,
) -> Result<Option<i64>, Box<dyn Error>> {
    let mut candidates = Vec::new();
    let mut stmt = conn.prepare(
        "SELECT pv.visitor_timestamp_id
         FROM pageviews pv
         WHERE pv.timestamp_id = pv.visitor_timestamp_id
           AND pv.path_id = ?1
           AND pv.visitor_timestamp_id >= ?2
           AND NOT EXISTS (
               SELECT 1
               FROM payments pay
               WHERE pay.visitor_timestamp_id = pv.visitor_timestamp_id
           )
           AND (
               SELECT MAX(later.timestamp_id)
               FROM pageviews later
               WHERE later.visitor_timestamp_id = pv.visitor_timestamp_id
           ) < ?3
         ORDER BY pv.visitor_timestamp_id DESC
         LIMIT 80",
    )?;
    let max_last_page_nanos = max_last_page_at * NANOS_PER_SECOND;
    let rows = stmt.query_map(
        params![home_path_id, min_visitor_timestamp_id, max_last_page_nanos],
        |row| row.get::<_, i64>(0),
    )?;
    for visitor_timestamp_id in rows {
        let visitor_timestamp_id = visitor_timestamp_id?;
        if !exclude.contains(&visitor_timestamp_id) {
            candidates.push(visitor_timestamp_id);
        }
    }
    Ok(candidates.choose(rng).copied())
}

fn insert_home_funnel_checkout_dropoffs(
    conn: &Connection,
    path_ids: &[i64],
    event_name_ids: &[i64],
    now: i64,
    generated_timestamp_ids: &mut HashSet<i64>,
    rng: &mut StdRng,
) -> Result<(), Box<dyn Error>> {
    // Keep drop-offs inside a typical "last 30 days" dashboard window.
    let min_visitor_timestamp_id = (now - 30 * DAY_SECONDS) * NANOS_PER_SECOND;
    // Leave room after the session's last page for the checkout path chain.
    let max_last_page_at = now - 1_800;
    let mut used = HashSet::new();
    let dropoffs = [
        HOME_FUNNEL_COMPLETED_SIGNUP_STEP,
        HOME_FUNNEL_STARTED_CHECKOUT_STEP,
    ];
    for &events_through in &dropoffs {
        let Some(visitor_timestamp_id) = pick_unconverted_home_landing(
            conn,
            path_ids[PATH_HOME],
            min_visitor_timestamp_id,
            max_last_page_at,
            &used,
            rng,
        )?
        else {
            return Err("not enough unconverted home landings for funnel drop-offs".into());
        };
        used.insert(visitor_timestamp_id);

        let last_page_at: i64 = conn.query_row(
            "SELECT MAX(timestamp_id) / ?2
             FROM pageviews
             WHERE visitor_timestamp_id = ?1",
            params![visitor_timestamp_id, NANOS_PER_SECOND],
            |row| row.get(0),
        )?;
        let journey_end_at = last_page_at + rng.gen_range(400..=1_200);

        extend_conversion_session(conn, visitor_timestamp_id, journey_end_at, rng)?;
        insert_home_funnel_checkout_journey(
            conn,
            path_ids,
            event_name_ids,
            visitor_timestamp_id,
            journey_end_at,
            events_through,
            generated_timestamp_ids,
            rng,
        )?;
    }
    Ok(())
}

fn clear_home_funnel_events(
    conn: &Connection,
    event_name_ids: &[i64],
    visitor_timestamp_id: i64,
) -> Result<(), Box<dyn Error>> {
    for &event_idx in HOME_FUNNEL_EVENT_INDICES.iter() {
        conn.execute(
            "DELETE FROM events
             WHERE event_name_id = ?1
               AND page_timestamp_id IN (
                   SELECT timestamp_id FROM pageviews WHERE visitor_timestamp_id = ?2
               )",
            params![event_name_ids[event_idx], visitor_timestamp_id],
        )?;
    }
    Ok(())
}

fn setup_home_conversion_funnel(
    conn: &Connection,
    path_ids: &[i64],
    event_name_ids: &[i64],
) -> Result<(), Box<dyn Error>> {
    conn.execute(
        "INSERT INTO funnels (name) VALUES (?1)",
        params![HOME_CONVERSION_FUNNEL_NAME],
    )?;
    let funnel_id = last_insert_rowid(conn)?;

    conn.execute(
        "INSERT INTO funnel_steps (funnel_id, step_type, path_id, is_landing, description, step_order)
         VALUES (?1, 'pageview', ?2, 1, ?3, 0)",
        params![funnel_id, path_ids[PATH_HOME], HOME_LANDING_GOAL_DESCRIPTION],
    )?;

    // Each event step is now a pageview step anchored to the page the event
    // fires on, with the event as the optional match on top of that page.
    for (event_step, description) in HOME_FUNNEL_EVENT_DESCRIPTIONS.iter().enumerate() {
        let event_name_id = event_name_ids[HOME_FUNNEL_EVENT_INDICES[event_step]];
        let path_id = path_ids[home_funnel_event_path_idx(event_step)];
        conn.execute(
            "INSERT INTO funnel_steps (funnel_id, step_type, path_id, event_name_id, is_landing, description, step_order)
             VALUES (?1, 'pageview', ?2, ?3, 0, ?4, ?5)",
            params![funnel_id, path_id, event_name_id, description, (event_step + 1) as i64],
        )?;
    }

    let payment_step_order = HOME_FUNNEL_EVENT_DESCRIPTIONS.len() as i64 + 1;
    conn.execute(
        "INSERT INTO funnel_steps (funnel_id, step_type, payment_types, is_landing, description, step_order)
         VALUES (?1, 'payment', 'single,subscription', 0, ?2, ?3)",
        params![funnel_id, "Completed payment", payment_step_order],
    )?;

    Ok(())
}

fn sync_landing_session_metrics(
    conn: &Connection,
    visitor_timestamp_id: i64,
) -> Result<(), Box<dyn Error>> {
    let (page_count, duration): (i64, i64) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(duration_seconds), 0)
         FROM pageviews
         WHERE visitor_timestamp_id = ?1",
        params![visitor_timestamp_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    conn.execute(
        "UPDATE landings
         SET session_duration_seconds = ?2,
             session_page_count = ?3
         WHERE visitor_timestamp_id = ?1",
        params![visitor_timestamp_id, duration, page_count],
    )?;
    Ok(())
}

fn emit_cancel_flow_opened(
    conn: &Connection,
    path_ids: &[i64],
    event_name_ids: &[i64],
    visitor_timestamp_id: i64,
    last_paid_at: i64,
    canceled_at: i64,
    generated_timestamp_ids: &mut HashSet<i64>,
    rng: &mut StdRng,
) -> Result<(), Box<dyn Error>> {
    let earliest = last_paid_at.saturating_add(MIN_CANCEL_FLOW_AFTER_PAYMENT_SECONDS);
    if earliest >= canceled_at {
        return Ok(());
    }

    let lead_seconds = rng.gen_range(3 * DAY_SECONDS..=21 * DAY_SECONDS);
    let latest = canceled_at.saturating_sub(lead_seconds).max(earliest);
    let session_start = if earliest == latest {
        earliest
    } else {
        rng.gen_range(earliest..=latest)
    };

    let previous_timestamp_id: i64 = conn.query_row(
        "SELECT timestamp_id FROM pageviews
         WHERE visitor_timestamp_id = ?1
         ORDER BY timestamp_id DESC
         LIMIT 1",
        params![visitor_timestamp_id],
        |row| row.get(0),
    )?;
    let page_timestamp_id = make_unique_timestamp_id(session_start, generated_timestamp_ids, rng);
    let mut session_pages = Vec::new();
    apply_pageview_duration_cap(
        conn,
        previous_timestamp_id,
        page_timestamp_id,
        &mut 0_i64,
        &mut session_pages,
    )?;

    let client_page_id = make_96bit_id(rng);
    let page_duration_seconds = rng.gen_range(45..=300);
    let page_scrolled_percent = rng.gen_range(20..=100);

    let page_metadata = maybe_page_metadata(rng)?;
    insert_pageview(
        conn,
        page_timestamp_id,
        visitor_timestamp_id,
        Some(previous_timestamp_id),
        path_ids[PATH_DASHBOARD],
        page_metadata.as_deref(),
        page_duration_seconds,
        page_scrolled_percent,
    )?;
    conn.execute(
        "INSERT INTO client_page_ids_map (visitor_timestamp_id, page_id)
         VALUES (?1, ?2)",
        params![page_timestamp_id, client_page_id],
    )?;

    if let Some(event_timestamp_id) = make_event_timestamp_id(
        page_timestamp_id,
        page_duration_seconds,
        None,
        generated_timestamp_ids,
        rng,
    ) {
        conn.execute(
            "INSERT INTO events (timestamp_id, event_name_id, page_timestamp_id, metadata)
             VALUES (?1, ?2, ?3, jsonb(?4))",
            params![
                event_timestamp_id,
                event_name_ids[CANCEL_FLOW_EVENT_INDEX],
                page_timestamp_id,
                event_metadata(rng)?,
            ],
        )?;
    }

    sync_landing_session_metrics(conn, visitor_timestamp_id)?;
    Ok(())
}

fn post_conversion_session_count(
    paid_at: i64,
    activity_end: i64,
    is_subscription: bool,
    rng: &mut StdRng,
) -> usize {
    if paid_at >= activity_end {
        return 0;
    }
    let remaining_days = ((activity_end - paid_at) / DAY_SECONDS).max(0) as usize;
    if remaining_days == 0 {
        return if rng.gen_bool(0.55) { 1 } else { 0 };
    }
    let max_sessions = remaining_days / 2 + 1;
    if is_subscription {
        rng.gen_range(2..=6).min(max_sessions.max(2))
    } else {
        rng.gen_range(1..=3).min(max_sessions.max(1))
    }
}

fn generate_post_conversion_activity(
    conn: &Connection,
    path_ids: &[i64],
    post_conversion_paths: &WeightedIndex<u32>,
    event_name_ids: &[i64],
    visitor_timestamp_id: i64,
    paid_at: i64,
    activity_end: i64,
    is_subscription: bool,
    generated_timestamp_ids: &mut HashSet<i64>,
    rng: &mut StdRng,
) -> Result<(), Box<dyn Error>> {
    if paid_at >= activity_end || !rng.gen_bool(POST_CONVERSION_RETURN_PROB) {
        return Ok(());
    }

    let mut previous_timestamp_id: i64 = conn.query_row(
        "SELECT timestamp_id FROM pageviews
         WHERE visitor_timestamp_id = ?1
         ORDER BY timestamp_id DESC
         LIMIT 1",
        params![visitor_timestamp_id],
        |row| row.get(0),
    )?;
    let mut last_page_at = session_start_seconds(previous_timestamp_id);
    let mut next_session_start = paid_at
        .max(last_page_at + 60)
        .saturating_add(rng.gen_range(300..=7_200));
    let session_count = post_conversion_session_count(paid_at, activity_end, is_subscription, rng);

    for _ in 0..session_count {
        if next_session_start >= activity_end {
            break;
        }
        let pages_in_session = if is_subscription {
            rng.gen_range(1..=5)
        } else {
            rng.gen_range(1..=3)
        };
        let mut session_pages: Vec<(i64, i64)> = Vec::with_capacity(pages_in_session);
        let mut page_offset_seconds = 0_i64;

        for _page_idx in 0..pages_in_session {
            let page_timestamp_seconds =
                (next_session_start + page_offset_seconds).min(activity_end);
            if page_timestamp_seconds <= last_page_at {
                page_offset_seconds += rng.gen_range(30..=180);
                continue;
            }
            let page_timestamp_id =
                make_unique_timestamp_id(page_timestamp_seconds, generated_timestamp_ids, rng);
            apply_pageview_duration_cap(
                conn,
                previous_timestamp_id,
                page_timestamp_id,
                &mut 0_i64,
                &mut session_pages,
            )?;
            let client_page_id = make_96bit_id(rng);
            let idx = post_conversion_paths.sample(rng);
            let path_idx = POST_CONVERSION_PATH_INDICES[idx];
            let path_id = path_ids[path_idx];
            let page_duration_seconds = rng.gen_range(15..=420);
            let page_scrolled_percent = rng.gen_range(10..=100);

            let page_metadata = maybe_page_metadata(rng)?;
            insert_pageview(
                conn,
                page_timestamp_id,
                visitor_timestamp_id,
                Some(previous_timestamp_id),
                path_id,
                page_metadata.as_deref(),
                page_duration_seconds,
                page_scrolled_percent,
            )?;
            conn.execute(
                "INSERT INTO client_page_ids_map (visitor_timestamp_id, page_id)
                 VALUES (?1, ?2)",
                params![page_timestamp_id, client_page_id],
            )?;

            maybe_emit_outbound_click_for_path(
                conn,
                event_name_ids,
                path_idx,
                page_timestamp_id,
                page_duration_seconds,
                generated_timestamp_ids,
                rng,
            )?;

            session_pages.push((page_timestamp_id, page_duration_seconds));
            previous_timestamp_id = page_timestamp_id;
            last_page_at = page_timestamp_seconds;
            page_offset_seconds += page_duration_seconds + rng.gen_range(5..=90);
        }

        let max_events = if is_subscription {
            rng.gen_range(1..=3)
        } else if rng.gen_bool(0.5) {
            1
        } else {
            0
        };
        let eligible_session_pages: Vec<(i64, i64)> = session_pages
            .into_iter()
            .filter(|(_, duration_seconds)| {
                duration_seconds * NANOS_PER_SECOND > NANOS_PER_SECOND + 1
            })
            .collect();
        for _ in 0..max_events {
            let Some(&(page_timestamp_id, duration_seconds)) = eligible_session_pages.choose(rng)
            else {
                break;
            };
            let Some(event_timestamp_id) = make_event_timestamp_id(
                page_timestamp_id,
                duration_seconds,
                None,
                generated_timestamp_ids,
                rng,
            ) else {
                continue;
            };
            let event_idx = *POST_CONVERSION_EVENT_INDICES
                .choose(rng)
                .expect("post-conversion event indices are non-empty");
            conn.execute(
                "INSERT INTO events (timestamp_id, event_name_id, page_timestamp_id, metadata)
                 VALUES (?1, ?2, ?3, jsonb(?4))",
                params![
                    event_timestamp_id,
                    event_name_ids[event_idx],
                    page_timestamp_id,
                    event_metadata(rng)?,
                ],
            )?;
        }

        let gap_days = rng
            .gen_range(1..=7)
            .min(((activity_end - next_session_start) / DAY_SECONDS).max(1) as i32)
            as i64;
        next_session_start = last_page_at
            .saturating_add(gap_days * DAY_SECONDS)
            .saturating_add(rng.gen_range(0..DAY_SECONDS));
    }

    Ok(())
}

fn pick_payment_day(budget: &[i64], rng: &mut StdRng) -> usize {
    let weights: Vec<u32> = budget.iter().map(|&amount| amount.max(1) as u32).collect();
    WeightedIndex::new(weights)
        .expect("day budget weights should be non-empty")
        .sample(rng)
}

fn schedule_payment_day(budget: &mut [i64], amount: i64, rng: &mut StdRng) -> usize {
    let day_idx = pick_payment_day(budget, rng);
    budget[day_idx] = budget[day_idx].saturating_sub(amount);
    day_idx
}

fn paid_at_for_day(month_start: i64, day_idx: usize, rng: &mut StdRng) -> i64 {
    month_start + day_idx as i64 * DAY_SECONDS + rng.gen_range(0..(DAY_SECONDS - 1))
}

fn pick_subscription_amount(remaining: i64, rng: &mut StdRng) -> i64 {
    let plans = [
        9_000, 12_000, 19_000, 29_000, 49_000, 79_000, 99_000, 149_000,
    ];
    if remaining <= 9_000 {
        remaining.max(1_000)
    } else {
        let best = plans
            .iter()
            .copied()
            .filter(|amount| *amount <= remaining + 8_000)
            .collect::<Vec<_>>();
        *best.choose(rng).unwrap_or(&19_000)
    }
}

fn make_96bit_id(rng: &mut StdRng) -> Vec<u8> {
    let mut value = vec![0_u8; 12];
    let ts = nanoseconds_since_epoch() as u32;
    value[0] = (ts >> 24) as u8;
    value[1] = (ts >> 16) as u8;
    value[2] = (ts >> 8) as u8;
    value[3] = ts as u8;
    rng.fill_bytes(&mut value[4..]);
    value
}

fn max_dwell_seconds_before_next_page(page_timestamp_id: i64, next_page_timestamp_id: i64) -> i64 {
    ((next_page_timestamp_id - page_timestamp_id) / NANOS_PER_SECOND).max(0)
}

fn cap_pageview_duration_before_next(
    conn: &Connection,
    page_timestamp_id: i64,
    next_page_timestamp_id: i64,
) -> Result<i64, Box<dyn Error>> {
    let max_dwell = max_dwell_seconds_before_next_page(page_timestamp_id, next_page_timestamp_id);
    let current_duration: i64 = conn.query_row(
        "SELECT COALESCE(duration_seconds, 0) FROM pageviews WHERE timestamp_id = ?1",
        params![page_timestamp_id],
        |row| row.get(0),
    )?;
    if current_duration <= max_dwell {
        return Ok(0);
    }
    conn.execute(
        "UPDATE pageviews SET duration_seconds = ?2 WHERE timestamp_id = ?1",
        params![page_timestamp_id, max_dwell],
    )?;
    Ok(current_duration - max_dwell)
}

fn apply_pageview_duration_cap(
    conn: &Connection,
    page_timestamp_id: i64,
    next_page_timestamp_id: i64,
    session_duration_seconds: &mut i64,
    session_pages: &mut [(i64, i64)],
) -> Result<(), Box<dyn Error>> {
    let duration_removed =
        cap_pageview_duration_before_next(conn, page_timestamp_id, next_page_timestamp_id)?;
    if duration_removed == 0 {
        return Ok(());
    }
    *session_duration_seconds -= duration_removed;
    if let Some((_, duration_seconds)) = session_pages
        .iter_mut()
        .find(|(id, _)| *id == page_timestamp_id)
    {
        *duration_seconds -= duration_removed;
    }
    Ok(())
}

fn make_unique_timestamp_id(
    timestamp_seconds: i64,
    seen: &mut HashSet<i64>,
    rng: &mut StdRng,
) -> i64 {
    loop {
        let timestamp_id =
            timestamp_seconds * NANOS_PER_SECOND + rng.gen_range(0..NANOS_PER_SECOND);
        if seen.insert(timestamp_id) {
            return timestamp_id;
        }
    }
}

fn make_event_timestamp_id(
    page_timestamp_id: i64,
    duration_seconds: i64,
    max_timestamp_cap: Option<i64>,
    seen: &mut HashSet<i64>,
    rng: &mut StdRng,
) -> Option<i64> {
    let min_timestamp_id = page_timestamp_id + NANOS_PER_SECOND + 1;
    let mut upper_bound = page_timestamp_id + duration_seconds * NANOS_PER_SECOND - 1;
    if let Some(cap) = max_timestamp_cap {
        upper_bound = upper_bound.min(cap);
    }
    if min_timestamp_id > upper_bound {
        return None;
    }
    for _ in 0..32 {
        let timestamp_id = rng.gen_range(min_timestamp_id..=upper_bound);
        if seen.insert(timestamp_id) {
            return Some(timestamp_id);
        }
    }
    None
}

/// IANA timezone, language, ISO country.
#[derive(Clone, Copy)]
struct LocaleSeed {
    timezone: &'static str,
    language: &'static str,
    country: &'static str,
}

/// Weighted visitor locales: existing US/GB/DE/FR/JP/IN/BR/AU mix plus broader global traffic.
const LOCALE_SEEDS: [LocaleSeed; 47] = [
    LocaleSeed {
        timezone: "America/New_York",
        language: "EN",
        country: "US",
    },
    LocaleSeed {
        timezone: "America/Chicago",
        language: "EN",
        country: "US",
    },
    LocaleSeed {
        timezone: "America/Denver",
        language: "EN",
        country: "US",
    },
    LocaleSeed {
        timezone: "America/Los_Angeles",
        language: "EN",
        country: "US",
    },
    LocaleSeed {
        timezone: "America/Toronto",
        language: "EN",
        country: "CA",
    },
    LocaleSeed {
        timezone: "America/Vancouver",
        language: "EN",
        country: "CA",
    },
    LocaleSeed {
        timezone: "America/Mexico_City",
        language: "ES",
        country: "MX",
    },
    LocaleSeed {
        timezone: "America/Argentina/Buenos_Aires",
        language: "ES",
        country: "AR",
    },
    LocaleSeed {
        timezone: "America/Santiago",
        language: "ES",
        country: "CL",
    },
    LocaleSeed {
        timezone: "America/Sao_Paulo",
        language: "PT",
        country: "BR",
    },
    LocaleSeed {
        timezone: "Europe/London",
        language: "EN",
        country: "GB",
    },
    LocaleSeed {
        timezone: "Europe/Dublin",
        language: "EN",
        country: "IE",
    },
    LocaleSeed {
        timezone: "Europe/Berlin",
        language: "DE",
        country: "DE",
    },
    LocaleSeed {
        timezone: "Europe/Paris",
        language: "FR",
        country: "FR",
    },
    LocaleSeed {
        timezone: "Europe/Amsterdam",
        language: "NL",
        country: "NL",
    },
    LocaleSeed {
        timezone: "Europe/Rome",
        language: "IT",
        country: "IT",
    },
    LocaleSeed {
        timezone: "Europe/Madrid",
        language: "ES",
        country: "ES",
    },
    LocaleSeed {
        timezone: "Europe/Stockholm",
        language: "SV",
        country: "SE",
    },
    LocaleSeed {
        timezone: "Europe/Oslo",
        language: "NO",
        country: "NO",
    },
    LocaleSeed {
        timezone: "Europe/Zurich",
        language: "DE",
        country: "CH",
    },
    LocaleSeed {
        timezone: "Europe/Vienna",
        language: "DE",
        country: "AT",
    },
    LocaleSeed {
        timezone: "Europe/Brussels",
        language: "FR",
        country: "BE",
    },
    LocaleSeed {
        timezone: "Europe/Warsaw",
        language: "PL",
        country: "PL",
    },
    LocaleSeed {
        timezone: "Europe/Lisbon",
        language: "PT",
        country: "PT",
    },
    LocaleSeed {
        timezone: "Europe/Copenhagen",
        language: "DA",
        country: "DK",
    },
    LocaleSeed {
        timezone: "Europe/Helsinki",
        language: "FI",
        country: "FI",
    },
    LocaleSeed {
        timezone: "Europe/Istanbul",
        language: "TR",
        country: "TR",
    },
    LocaleSeed {
        timezone: "Asia/Dubai",
        language: "EN",
        country: "AE",
    },
    LocaleSeed {
        timezone: "Asia/Riyadh",
        language: "AR",
        country: "SA",
    },
    LocaleSeed {
        timezone: "Asia/Jerusalem",
        language: "HE",
        country: "IL",
    },
    LocaleSeed {
        timezone: "Asia/Qatar",
        language: "AR",
        country: "QA",
    },
    LocaleSeed {
        timezone: "Africa/Cairo",
        language: "AR",
        country: "EG",
    },
    LocaleSeed {
        timezone: "Asia/Tokyo",
        language: "JA",
        country: "JP",
    },
    LocaleSeed {
        timezone: "Asia/Kolkata",
        language: "EN",
        country: "IN",
    },
    LocaleSeed {
        timezone: "Asia/Shanghai",
        language: "ZH",
        country: "CN",
    },
    LocaleSeed {
        timezone: "Asia/Hong_Kong",
        language: "ZH",
        country: "HK",
    },
    LocaleSeed {
        timezone: "Asia/Taipei",
        language: "ZH",
        country: "TW",
    },
    LocaleSeed {
        timezone: "Asia/Singapore",
        language: "EN",
        country: "SG",
    },
    LocaleSeed {
        timezone: "Asia/Seoul",
        language: "KO",
        country: "KR",
    },
    LocaleSeed {
        timezone: "Asia/Jakarta",
        language: "ID",
        country: "ID",
    },
    LocaleSeed {
        timezone: "Asia/Manila",
        language: "EN",
        country: "PH",
    },
    LocaleSeed {
        timezone: "Pacific/Auckland",
        language: "EN",
        country: "NZ",
    },
    LocaleSeed {
        timezone: "Australia/Sydney",
        language: "EN",
        country: "AU",
    },
    LocaleSeed {
        timezone: "Australia/Melbourne",
        language: "EN",
        country: "AU",
    },
    LocaleSeed {
        timezone: "Africa/Johannesburg",
        language: "EN",
        country: "ZA",
    },
    LocaleSeed {
        timezone: "Africa/Lagos",
        language: "EN",
        country: "NG",
    },
    LocaleSeed {
        timezone: "Africa/Nairobi",
        language: "EN",
        country: "KE",
    },
];

const LOCALE_WEIGHTS: [u32; 47] = [
    95, 75, 45, 95, // US
    55, 25, // CA
    38, 22, 18, // MX, AR, CL
    45, // BR
    70, // GB
    28, // IE
    65, 50, // DE, FR
    35, 45, 40, 28, 22, 25, 22, 20, 32, 22, 18, 18, // Europe
    38, // TR
    28, 22, 24, 14, 16, // Middle East
    40, 55, // JP, IN
    42, 16, 18, 28, 32, 20, 16, // East / SE Asia
    24, // NZ
    35, 20, // AU
    32, 28, 12, // Africa
];

struct LocalePickers {
    locales: WeightedIndex<u32>,
}

impl LocalePickers {
    fn new() -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            locales: WeightedIndex::new(LOCALE_WEIGHTS)?,
        })
    }

    fn pick(&self, rng: &mut StdRng) -> LocaleSeed {
        LOCALE_SEEDS[self.locales.sample(rng)]
    }
}

#[allow(clippy::type_complexity)]
/// Worldwide operating system share (May 2026): Android 34.9%, Windows 29.98%, iOS 16.81%, …
const OS_SHARE_ANDROID: f64 = 34.9;
const OS_SHARE_WINDOWS: f64 = 29.98;
const OS_SHARE_IOS: f64 = 16.81;
const OS_SHARE_UNKNOWN: f64 = 9.06;
/// OS X + macOS (stored as `macos` in landings).
const OS_SHARE_MACOS: f64 = 7.03;
const OS_SHARE_LINUX: f64 = 0.74;
const OS_SHARE_CHROMEOS: f64 = 0.74;
const OS_SHARE_KAIOS: f64 = 0.74;

const OS_PICK_WEIGHTS: &[f64] = &[
    OS_SHARE_ANDROID,
    OS_SHARE_WINDOWS,
    OS_SHARE_IOS,
    OS_SHARE_UNKNOWN,
    OS_SHARE_MACOS,
    OS_SHARE_LINUX,
    OS_SHARE_CHROMEOS,
    OS_SHARE_KAIOS,
];

struct OsPickers {
    systems: WeightedIndex<u32>,
}

impl OsPickers {
    fn new() -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            systems: weighted_index_from_f64(OS_PICK_WEIGHTS)?,
        })
    }

    fn pick_profile(&self, rng: &mut StdRng) -> (Option<&'static str>, Option<f64>, &'static str) {
        match self.systems.sample(rng) {
            0 => (Some("android"), Some(rng.gen_range(12.0..=18.0)), "mobile"),
            1 => (
                Some("windows"),
                Some(rng.gen_range(10.0..=14.0)),
                if rng.gen_bool(0.06) {
                    "desktop_touch"
                } else {
                    "desktop"
                },
            ),
            2 => (
                Some("ios"),
                Some(rng.gen_range(13.0..=17.0)),
                if rng.gen_bool(0.82) {
                    "mobile"
                } else {
                    "tablet"
                },
            ),
            3 => (None, None, pick_unknown_device(rng)),
            4 => (Some("macos"), Some(rng.gen_range(13.0..=15.0)), "desktop"),
            5 => (Some("linux"), Some(rng.gen_range(5.0..=6.8)), "desktop"),
            6 => (
                Some("chromeos"),
                Some(rng.gen_range(120.0..=130.0)),
                "desktop",
            ),
            7 => (Some("kaios"), Some(rng.gen_range(2.0..=3.5)), "mobile"),
            _ => unreachable!("OS picker index out of range"),
        }
    }
}

fn pick_unknown_device(rng: &mut StdRng) -> &'static str {
    match rng.gen_range(0..100) {
        0..=54 => "mobile",
        55..=84 => "desktop",
        _ => "tablet",
    }
}

/// Worldwide browser family share (May 2026): Chrome 70.25%, Safari 15.72%, Edge 5.14%, …
const BROWSER_SHARE_CHROME: f64 = 70.25;
const BROWSER_SHARE_EDGE: f64 = 5.14;
const BROWSER_SHARE_FIREFOX: f64 = 2.19;
const BROWSER_SHARE_SAMSUNG: f64 = 1.89;
const BROWSER_SHARE_OPERA: f64 = 1.78;
/// Remaining families each below 1% (brave, duckduckgo, ie, uc, qq, baidu, yandex, kai).
const BROWSER_SHARE_BRAVE: f64 = 0.55;
const BROWSER_SHARE_DUCKDUCKGO: f64 = 0.55;
const BROWSER_SHARE_IE: f64 = 0.45;
const BROWSER_SHARE_UC: f64 = 0.35;
const BROWSER_SHARE_QQ: f64 = 0.30;
const BROWSER_SHARE_BAIDU: f64 = 0.30;
const BROWSER_SHARE_YANDEX: f64 = 0.28;

const WINDOWS_BROWSERS: &[&str] = &[
    "chrome",
    "edge",
    "firefox",
    "opera",
    "brave",
    "duckduckgo",
    "ie",
    "yandex",
];
const WINDOWS_BROWSER_WEIGHTS: &[f64] = &[
    BROWSER_SHARE_CHROME,
    BROWSER_SHARE_EDGE,
    BROWSER_SHARE_FIREFOX,
    BROWSER_SHARE_OPERA,
    BROWSER_SHARE_BRAVE,
    BROWSER_SHARE_DUCKDUCKGO,
    BROWSER_SHARE_IE,
    BROWSER_SHARE_YANDEX,
];

const MACOS_BROWSERS: &[&str] = &["safari", "chrome", "edge", "firefox", "opera", "brave"];
/// macOS desktop skews Safari-heavy so global Safari share is reachable without overstating iOS.
const MACOS_BROWSER_WEIGHTS: &[f64] = &[82.0, 10.0, 3.0, 2.5, 1.5, 1.0];

const ANDROID_BROWSERS: &[&str] = &[
    "chrome",
    "edge",
    "firefox",
    "samsung",
    "opera",
    "brave",
    "duckduckgo",
    "uc",
    "qq",
    "baidu",
];
const ANDROID_BROWSER_WEIGHTS: &[f64] = &[
    BROWSER_SHARE_CHROME,
    BROWSER_SHARE_EDGE,
    BROWSER_SHARE_FIREFOX,
    BROWSER_SHARE_SAMSUNG,
    BROWSER_SHARE_OPERA,
    BROWSER_SHARE_BRAVE,
    BROWSER_SHARE_DUCKDUCKGO,
    BROWSER_SHARE_UC,
    BROWSER_SHARE_QQ,
    BROWSER_SHARE_BAIDU,
];

const IOS_BROWSERS: &[&str] = &["safari", "chrome", "edge", "firefox", "opera"];
/// iOS: Safari default; Chrome and Edge are alternate WebKit shells.
const IOS_BROWSER_WEIGHTS: &[f64] = &[88.0, 7.0, 3.0, 1.5, 0.5];

struct BrowserPickers {
    windows: WeightedIndex<u32>,
    macos: WeightedIndex<u32>,
    android: WeightedIndex<u32>,
    ios: WeightedIndex<u32>,
}

impl BrowserPickers {
    fn new() -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            windows: weighted_index_from_f64(WINDOWS_BROWSER_WEIGHTS)?,
            macos: weighted_index_from_f64(MACOS_BROWSER_WEIGHTS)?,
            android: weighted_index_from_f64(ANDROID_BROWSER_WEIGHTS)?,
            ios: weighted_index_from_f64(IOS_BROWSER_WEIGHTS)?,
        })
    }

    fn pick(&self, os: &str, rng: &mut StdRng) -> &'static str {
        match os {
            "windows" => WINDOWS_BROWSERS[self.windows.sample(rng)],
            "macos" => MACOS_BROWSERS[self.macos.sample(rng)],
            "android" => ANDROID_BROWSERS[self.android.sample(rng)],
            "ios" => IOS_BROWSERS[self.ios.sample(rng)],
            _ => WINDOWS_BROWSERS[self.windows.sample(rng)],
        }
    }
}

fn weighted_index_from_f64(weights: &[f64]) -> Result<WeightedIndex<u32>, Box<dyn Error>> {
    let scaled: Vec<u32> = weights
        .iter()
        .map(|weight| ((*weight * 100.0).round() as u32).max(1))
        .collect();
    Ok(WeightedIndex::new(scaled)?)
}

fn random_device_identity(
    locale_pickers: &LocalePickers,
    os_pickers: &OsPickers,
    browser_pickers: &BrowserPickers,
    rng: &mut StdRng,
) -> (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    Option<&'static str>,
    Option<f64>,
    Option<&'static str>,
    &'static str,
) {
    let locale = locale_pickers.pick(rng);
    let (tz, language, country) = (locale.timezone, locale.language, locale.country);
    let cities = major_cities_for_country(country)
        .expect("every generate locale country should have major cities");
    let city = cities[rng.gen_range(0..cities.len())];

    let (os, os_version, device) = os_pickers.pick_profile(rng);
    let browser_family = os.map(|slug| browser_pickers.pick(slug, rng));
    (
        tz,
        language,
        country,
        city,
        os,
        os_version,
        browser_family,
        device,
    )
}

/// Width-bucket slugs; ranges match `width_bucket` in `revscope/src/audience/mod.rs`.
const SLUGS_MOBILE: &[&'static str] = &["small_mobile", "mobile"];
const SLUGS_MOBILE_WEIGHTS: &[u32] = &[22, 78];

const SLUGS_TABLET: &[&'static str] = &["tablet"];
const SLUGS_TABLET_WEIGHTS: &[u32] = &[100];

const SLUGS_DESKTOP: &[&'static str] = &["desktop", "large_desktop", "x_large_desktop"];
const SLUGS_DESKTOP_WEIGHTS: &[u32] = &[32, 43, 25];

const SCREEN_SMALL_MOBILE: &[(i64, i64)] = &[
    (320, 568),
    (320, 640),
    (360, 640),
    (360, 780),
    (375, 667),
    (375, 812),
    (390, 844),
];
const SCREEN_MOBILE: &[(i64, i64)] = &[
    (360, 640),
    (360, 800),
    (390, 844),
    (393, 873),
    (412, 915),
    (414, 896),
    (428, 926),
    (430, 932),
];
const SCREEN_TABLET: &[(i64, i64)] = &[
    (768, 1024),
    (800, 1280),
    (810, 1080),
    (820, 1180),
    (834, 1194),
    (1024, 1366),
    (1080, 1350),
];
const SCREEN_LAPTOP: &[(i64, i64)] = &[
    (1024, 768),
    (1280, 720),
    (1280, 800),
    (1366, 768),
    (1440, 900),
    (1536, 864),
    (1600, 900),
];
const SCREEN_LARGE_DESKTOP: &[(i64, i64)] = &[
    (1680, 1050),
    (1792, 1120),
    (1920, 1080),
    (1920, 1200),
    (1600, 1200),
    (2560, 1080),
];
const SCREEN_X_LARGE: &[(i64, i64)] = &[
    (1920, 1080),
    (2560, 1440),
    (2560, 1600),
    (3440, 1440),
    (3840, 2160),
    (5120, 2880),
];

fn pick_weighted_slug(slugs: &[&'static str], weights: &[u32], rng: &mut StdRng) -> &'static str {
    let total: u32 = weights.iter().sum();
    let mut pick = rng.gen_range(0..total);
    for (index, weight) in weights.iter().enumerate() {
        if pick < *weight {
            return slugs[index];
        }
        pick -= weight;
    }
    slugs[slugs.len() - 1]
}

/// Picks a width bucket appropriate for the device class (used for both screen and viewport).
fn pick_width_slug_for_device(device: &str, rng: &mut StdRng) -> &'static str {
    match device {
        "mobile" => pick_weighted_slug(SLUGS_MOBILE, SLUGS_MOBILE_WEIGHTS, rng),
        "tablet" => pick_weighted_slug(SLUGS_TABLET, SLUGS_TABLET_WEIGHTS, rng),
        "desktop" | "desktop_touch" => {
            pick_weighted_slug(SLUGS_DESKTOP, SLUGS_DESKTOP_WEIGHTS, rng)
        }
        _ => pick_weighted_slug(SLUGS_MOBILE, SLUGS_MOBILE_WEIGHTS, rng),
    }
}

/// Viewport width range (min inclusive, max exclusive) for a slug.
fn viewport_width_range(slug: &str) -> (i64, i64) {
    match slug {
        "small_mobile" => (280, 360),
        "mobile" => (360, 768),
        "tablet" => (768, 1024),
        "desktop" => (1024, 1440),
        "large_desktop" => (1440, 1920),
        "x_large_desktop" => (1920, 2561),
        _ => (1024, 1440),
    }
}

fn min_viewport_height(slug: &str) -> i64 {
    match slug {
        "small_mobile" => 480,
        "mobile" => 640,
        "tablet" => 700,
        "desktop" => 600,
        "large_desktop" => 800,
        "x_large_desktop" => 900,
        _ => 600,
    }
}

/// Viewport width in the same bucket as `slug`, not wider than `screen_w`.
fn random_viewport_width(slug: &str, screen_w: i64, rng: &mut StdRng) -> i64 {
    let (min_w, max_w) = viewport_width_range(slug);
    let upper = screen_w.min(max_w - 1);
    if upper <= min_w {
        min_w
    } else {
        rng.gen_range(min_w..=upper)
    }
}

fn random_viewport_height(
    slug: &str,
    device: &str,
    screen_h: i64,
    viewport_w: i64,
    rng: &mut StdRng,
) -> i64 {
    let min_h = min_viewport_height(slug);
    match device {
        "mobile" | "tablet" => {
            let slack = screen_h - min_h;
            if slack <= 40 {
                return min_h;
            }
            let chrome = rng.gen_range(40..=slack.min(200));
            (screen_h - chrome).max(min_h)
        }
        _ => {
            let aspect = rng.gen_range(1.45..1.9);
            let from_aspect = (viewport_w as f64 * aspect).round() as i64;
            from_aspect.min(screen_h.saturating_sub(72)).max(min_h)
        }
    }
}

fn resolutions_for_slug(slug: &str) -> &'static [(i64, i64)] {
    match slug {
        "small_mobile" => SCREEN_SMALL_MOBILE,
        "mobile" => SCREEN_MOBILE,
        "tablet" => SCREEN_TABLET,
        "desktop" => SCREEN_LAPTOP,
        "large_desktop" => SCREEN_LARGE_DESKTOP,
        "x_large_desktop" => SCREEN_X_LARGE,
        _ => SCREEN_LAPTOP,
    }
}

fn random_screen_profile(device: &str, rng: &mut StdRng) -> (i64, i64, i64, i64) {
    let slug = pick_width_slug_for_device(device, rng);
    let (screen_w, screen_h) = *resolutions_for_slug(slug)
        .choose(rng)
        .expect("screen resolution list should not be empty");
    let viewport_w = random_viewport_width(slug, screen_w, rng);
    let viewport_h = random_viewport_height(slug, device, screen_h, viewport_w, rng);
    (screen_w, screen_h.max(viewport_h), viewport_w, viewport_h)
}

fn insert_risk_payment(
    conn: &Connection,
    ids: &mut PaymentIdState,
    active_subscriptions: &[ActiveSubscription],
    month_first_pages: &[i64],
    month_start: i64,
    month_end: i64,
    payment_type: &str,
    rng: &mut StdRng,
) -> Result<(), Box<dyn Error>> {
    let amount = -rng.gen_range(2_000..=45_000);
    let paid_at = rng.gen_range(month_start..=month_end);
    let subscription = active_subscriptions.choose(rng);
    let maybe_subscription_id = subscription.map(|s| s.id);
    let interval_unit = subscription.map(|s| s.interval_unit);
    let visitor_timestamp_id = month_first_pages
        .choose(rng)
        .expect("month should include sessions")
        .to_owned();
    let customer_idx = rng.gen_range(10_000..=45_000);

    insert_payment(
        conn,
        ids,
        maybe_subscription_id,
        interval_unit,
        Some(visitor_timestamp_id),
        &format!("cus_risk_{customer_idx}"),
        &format!("{payment_type}{customer_idx}@acme-example.com"),
        payment_type,
        amount,
        None,
        None,
        paid_at,
        json!({
            "reason": if payment_type == "refund" { "requested" } else { "disputed" },
            "risk_score": rng.gen_range(10..=98)
        })
        .to_string(),
        rng,
    )?;
    Ok(())
}

fn session_metrics_at_payment(
    conn: &Connection,
    visitor_timestamp_id: i64,
    paid_at: i64,
) -> Result<(Option<i64>, Option<i64>), Box<dyn Error>> {
    let paid_at_nanos = paid_at.saturating_mul(NANOS_PER_SECOND);
    let (page_count, duration): (i64, i64) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(duration_seconds), 0)
         FROM pageviews
         WHERE visitor_timestamp_id = ?1
           AND timestamp_id <= ?2",
        params![visitor_timestamp_id, paid_at_nanos],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if page_count == 0 {
        Ok((None, None))
    } else {
        Ok((Some(duration), Some(page_count)))
    }
}

#[allow(clippy::too_many_arguments)]
fn insert_payment(
    conn: &Connection,
    ids: &mut PaymentIdState,
    subscription_id: Option<i64>,
    interval_unit: Option<&str>,
    visitor_timestamp_id: Option<i64>,
    provider_customer_id: &str,
    customer_email: &str,
    payment_type: &str,
    amount: i64,
    fee_amount: Option<i64>,
    net_amount: Option<i64>,
    paid_at: i64,
    metadata: String,
    rng: &mut StdRng,
) -> Result<(), Box<dyn Error>> {
    let provider_payment_id = format!("pay_acme_{:09}", ids.payment_seq);
    ids.payment_seq += 1;
    let provider_invoice_id = if payment_type == "single" || payment_type == "subscription" {
        let value = format!("in_acme_{:09}", ids.invoice_seq);
        ids.invoice_seq += 1;
        Some(value)
    } else {
        None
    };
    let provider_order_id = if payment_type == "single" {
        let value = format!("ord_acme_{:09}", ids.order_seq);
        ids.order_seq += 1;
        Some(value)
    } else {
        None
    };

    let created_at = (paid_at - rng.gen_range(10..=7200)).max(0);
    let status = if payment_type == "subscription" && rng.gen_bool(0.02) {
        "pending"
    } else if (payment_type == "single" || payment_type == "subscription") && rng.gen_bool(0.005) {
        "failed"
    } else if (payment_type == "single" || payment_type == "subscription") && rng.gen_bool(0.003) {
        "canceled"
    } else {
        "ok"
    };

    let (session_duration_seconds, session_page_count) =
        if let Some(visitor_timestamp_id) = visitor_timestamp_id {
            session_metrics_at_payment(conn, visitor_timestamp_id, paid_at)?
        } else {
            (None, None)
        };

    conn.execute(
        "INSERT INTO payments (
            provider, subscription_id, interval_unit, visitor_timestamp_id,
            attributed_visitor_timestamp_id, session_duration_seconds, session_page_count,
            provider_payment_id, provider_order_id, provider_invoice_id, provider_customer_id,
            customer_email, payment_type, status, amount, mrr_amount, currency, fee_amount, net_amount,
            paid_at, created_at, metadata
         ) VALUES (
            'stripe', ?1, ?2, ?3,
            ?4, ?5, ?6,
            ?7, ?8, ?9, ?10,
            ?11, ?12, ?13, ?14,
            CASE
                WHEN ?12 = 'subscription' AND ?2 IS NOT NULL THEN (
                    CASE ?2
                        WHEN 'month' THEN ?14
                        WHEN 'year' THEN ?14 / 12
                        WHEN 'week' THEN ?14 * 52 / 12
                        WHEN 'day' THEN CAST(?14 * 365.25 / 12 AS INTEGER)
                        ELSE NULL
                    END
                )
                ELSE NULL
            END,
            'USD', ?15, ?16,
            ?17, ?18, ?19
         )",
        params![
            subscription_id,
            interval_unit,
            visitor_timestamp_id,
            visitor_timestamp_id,
            session_duration_seconds,
            session_page_count,
            provider_payment_id,
            provider_order_id,
            provider_invoice_id,
            provider_customer_id,
            customer_email,
            payment_type,
            status,
            amount,
            fee_amount,
            net_amount,
            paid_at,
            created_at,
            metadata,
        ],
    )?;

    Ok(())
}

/// SQLite WAL companions of `{data_dir}/{project_id}.db`: the log and its index.
fn destination_db_files(data_dir: &str, project_id: &str) -> [PathBuf; 3] {
    let dir = Path::new(data_dir);
    [
        dir.join(format!("{project_id}.db")),
        dir.join(format!("{project_id}.db-wal")),
        dir.join(format!("{project_id}.db-shm")),
    ]
}

fn remove_destination_db_files(data_dir: &str, project_id: &str) -> Result<(), Box<dyn Error>> {
    println!("generate: removing existing database files for project {project_id}…");
    for path in destination_db_files(data_dir, project_id) {
        match std::fs::remove_file(&path) {
            Ok(()) => println!("generate: removed {}", path.display()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!("failed to remove {}: {error}", path.display()).into());
            }
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let usage = "usage: cargo run --features generate --bin generate -- <project-id> [data-dir] [--empty] [--overwrite]\n\
         example: cargo run --features generate --bin generate -- 0 0\n\
         empty:   cargo run --features generate --bin generate -- 2 data --empty\n\
         overwrite: cargo run --features generate --bin generate -- 1 --overwrite";
    let mut empty = false;
    let mut overwrite = false;
    let mut positionals = Vec::new();
    for arg in std::env::args().skip(1) {
        if arg == "--empty" {
            empty = true;
        } else if arg == "--overwrite" {
            overwrite = true;
        } else if arg.starts_with('-') {
            return Err(format!("unknown flag {arg}\n{usage}").into());
        } else {
            positionals.push(arg);
        }
    }
    let mut positionals = positionals.into_iter();
    let project_id = positionals.next().ok_or(usage)?;
    let data_dir = positionals.next().unwrap_or_else(|| "data".to_string());
    if positionals.next().is_some() {
        return Err(format!("too many arguments\n{usage}").into());
    }
    run(&project_id, &data_dir, empty, overwrite)
}

#[cfg(test)]
mod tests {
    use super::destination_db_files;
    use super::remove_destination_db_files;

    #[test]
    fn overwrite_removes_db_wal_and_index() {
        let dir = std::env::temp_dir().join(format!(
            "revscope-generate-overwrite-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let data_dir = dir.to_str().expect("utf8 temp path");
        for path in destination_db_files(data_dir, "9") {
            std::fs::write(&path, b"x").expect("seed file");
        }

        remove_destination_db_files(data_dir, "9").expect("remove");

        for path in destination_db_files(data_dir, "9") {
            assert!(!path.exists(), "{} should be gone", path.display());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
