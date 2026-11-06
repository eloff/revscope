//! Shared SQL for attributed landing-visit revenue lookups.

/// Exclude subscription renewal invoices; only the first successful charge counts.
pub(crate) const EXCLUDE_SUBSCRIPTION_RENEWALS: &str = "NOT (
    payment_type = 'subscription'
    AND EXISTS (
        SELECT 1
        FROM payments prior
        WHERE prior.subscription_id = payments.subscription_id
          AND prior.status = 'ok'
          AND prior.payment_type = 'subscription'
          AND prior.paid_at < payments.paid_at
          AND prior.id != payments.id
    )
)";

/// CTE `page_revenue(visitor_timestamp_id, amount)` for successful payments
/// attributed to visits in `[?start_param, ?end_param)`.
///
/// Join to landings on `visitor_timestamp_id`. The attributed-visit bounds must
/// match the landings window so the aggregate only covers rows that can join.
/// When `exclude_renewals` is true, only the first successful subscription
/// charge per subscription is included.
pub(crate) fn page_revenue_cte(start_param: u32, end_param: u32, exclude_renewals: bool) -> String {
    format!(
        "page_revenue AS (\n{}\n)",
        page_revenue_select(start_param, end_param, exclude_renewals)
    )
}

/// The query behind [`page_revenue_cte`], one row per visit in visit order.
/// Run on its own it streams off the attributed-visit index, where a joined
/// CTE is materialized whole first.
pub(crate) fn page_revenue_select(
    start_param: u32,
    end_param: u32,
    exclude_renewals: bool,
) -> String {
    let renewal_filter = if exclude_renewals {
        format!("\n      AND {EXCLUDE_SUBSCRIPTION_RENEWALS}")
    } else {
        String::new()
    };
    format!(
        "    SELECT attributed_visitor_timestamp_id AS visitor_timestamp_id, SUM(amount) AS amount
    FROM payments
    WHERE status = 'ok'
      AND payment_type IN ('single', 'subscription')
      AND attributed_visitor_timestamp_id IS NOT NULL
      AND attributed_visitor_timestamp_id >= ?{start_param}
      AND attributed_visitor_timestamp_id < ?{end_param}{renewal_filter}
    GROUP BY attributed_visitor_timestamp_id
    ORDER BY attributed_visitor_timestamp_id"
    )
}
