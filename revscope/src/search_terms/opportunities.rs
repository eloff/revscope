//! Flagging the terms most worth attention: clicks lost to a click-through
//! rate below the site's own curve, and clicks split across landing pages.

use std::cmp::Ordering;

use crate::query_params::cmp_f64;

use super::rows::{RawTerm, click_through_rate};

/// How many rows to flag for cannibalization and CTR-gap highlights.
pub(super) const HIGHLIGHT_TOP_N: usize = 3;
pub(super) const MIN_RANK: f64 = 1.0;
pub(super) const MAX_RANK: f64 = 10.0;

/// Impression-weighted CTR for integer ranks 1..=10 from terms that received
/// at least one click. Zero-click queries are excluded so the long tail does
/// not pull expected CTR toward 0. Empty ranks stay `None`.
pub(super) fn rank_ctr_curve(terms: &[RawTerm]) -> [Option<f64>; 11] {
    let mut clicks = [0_i64; 11];
    let mut impressions = [0_i64; 11];
    for term in terms {
        if term.clicks <= 0 {
            continue;
        }
        let rank = term.position.round() as i32;
        if (1..=10).contains(&rank) {
            let index = rank as usize;
            clicks[index] += term.clicks;
            impressions[index] += term.impressions;
        }
    }
    let mut curve = [None; 11];
    for rank in 1..=10 {
        if impressions[rank] > 0 {
            curve[rank] = Some(clicks[rank] as f64 / impressions[rank] as f64);
        }
    }
    curve
}

/// Typical CTR for `position`, interpolating adjacent rank buckets. `None`
/// when the position is outside 1..=10 or neither neighboring bucket has data.
pub(super) fn expected_ctr_at(position: f64, curve: &[Option<f64>; 11]) -> Option<f64> {
    if !(MIN_RANK..=MAX_RANK).contains(&position) {
        return None;
    }
    let lo = position.floor() as usize;
    let hi = position.ceil() as usize;
    let frac = position - lo as f64;
    match (curve[lo], curve[hi]) {
        (Some(low), Some(high)) => Some(low + (high - low) * frac),
        (Some(low), None) => Some(low),
        (None, Some(high)) => Some(high),
        (None, None) => None,
    }
}

/// Writes expected CTR, lost clicks, and lost revenue for every term using the
/// site-own rank curve and the channel's revenue per click.
pub(super) fn attach_ctr_opportunities(
    terms: &mut [RawTerm],
    channel_revenue: i64,
    total_clicks: i64,
) {
    let curve = rank_ctr_curve(terms);
    let revenue_per_click = if total_clicks > 0 {
        channel_revenue as f64 / total_clicks as f64
    } else {
        0.0
    };
    for term in terms.iter_mut() {
        let Some(expected) = expected_ctr_at(term.position, &curve) else {
            term.expected_ctr = 0.0;
            term.lost_clicks = 0.0;
            term.lost_revenue = 0.0;
            continue;
        };
        let actual = click_through_rate(term.clicks, term.impressions);
        let lost_clicks = if term.impressions > 0 {
            term.impressions as f64 * (expected - actual).max(0.0)
        } else {
            0.0
        };
        term.expected_ctr = expected;
        term.lost_clicks = lost_clicks;
        term.lost_revenue = lost_clicks * revenue_per_click;
    }
}

/// Flags the top [`HIGHLIGHT_TOP_N`] terms by lost clicks and by cannibalized
/// clicks. Ties at the cutoff all receive the flag. Rank-bucket expected CTR
/// is already baked into `lost_clicks`; the argmax is global, not per rank.
/// Zero-click terms can have a large CTR gap but no traffic to improve, so
/// they are not flagged.
pub(super) fn mark_search_term_highlights(terms: &mut [RawTerm]) {
    flag_top_n(
        terms,
        HIGHLIGHT_TOP_N,
        |term| {
            if term.clicks > 0 {
                term.lost_clicks
            } else {
                0.0
            }
        },
        |term| term.highest_lost_clicks = true,
    );
    flag_top_n(
        terms,
        HIGHLIGHT_TOP_N,
        |term| term.cannibalized_clicks as f64,
        |term| term.highest_cannibalization = true,
    );
}

/// Moves highlighted rows to the front so they are visible on page 1 of any
/// sort, then leaves the remaining rows in their existing order.
pub(super) fn promote_highlighted_terms(terms: &mut Vec<RawTerm>) {
    let mut attention = Vec::new();
    let mut rest = Vec::new();
    for term in terms.drain(..) {
        if term.highest_lost_clicks || term.highest_cannibalization {
            attention.push(term);
        } else {
            rest.push(term);
        }
    }
    attention.sort_by(|left, right| {
        cmp_f64(right.lost_clicks, left.lost_clicks)
            .then_with(|| right.cannibalized_clicks.cmp(&left.cannibalized_clicks))
            .then_with(|| left.query.cmp(&right.query))
    });
    attention.append(&mut rest);
    *terms = attention;
}

/// Sets a flag on every row whose `value` is among the top `n` positive
/// values. Rows that tie the nth score are all flagged.
pub(super) fn flag_top_n(
    terms: &mut [RawTerm],
    n: usize,
    value: impl Fn(&RawTerm) -> f64,
    mut set_flag: impl FnMut(&mut RawTerm),
) {
    let mut scores: Vec<f64> = terms
        .iter()
        .map(&value)
        .filter(|candidate| *candidate > 0.0)
        .collect();
    if scores.is_empty() || n == 0 {
        return;
    }
    scores.sort_by(|left, right| right.partial_cmp(left).unwrap_or(Ordering::Equal));
    let threshold = scores[n.min(scores.len()) - 1];
    for term in terms.iter_mut() {
        if value(term) >= threshold && value(term) > 0.0 {
            set_flag(term);
        }
    }
}
