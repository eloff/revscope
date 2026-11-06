//! Global segment filters shared across analytics routes.
//!
//! A segment filter narrows every analytics query to a subset of traffic.
//! Multiple values of the same dimension are combined with `OR`; different
//! dimensions are combined with `AND`. The dimensions are:
//!
//! - **country** — `landings.country` (ISO 3166-1 alpha-2, uppercase), applied
//!   as an `IN (...)` clause on the landing row.
//! - **source** — the session's revenue source, derived from
//!   `landings.campaign` / `landings.referrer` (a campaign, a plain referrer,
//!   or the direct bucket).
//! - **page** — restricts to sessions that contain a pageview of a selected
//!   page (`pageviews.path_id`).
//! - **landing page** — restricts to sessions whose first pageview (the landing
//!   pageview, whose `timestamp_id` equals the session id) is a selected page.
//! - **event** — restricts to sessions that contain an event with a selected
//!   name (`events.event_name_id`), reached through the event's pageview.
//! - **landing attributes** — channel, region, city, browser, OS and device,
//!   each a property of the landing row. See [`LandingAttribute`].
//!
//! Each route's query parser calls [`parse_segment_filters`] on the request
//! target and stores the result on its `*Query` struct. When building SQL the
//! route appends the country clause via [`SegmentFilters::country_in_clause`]
//! (for queries already scanning `landings`) or
//! [`SegmentFilters::country_exists_clause`] (for queries on `pageviews`,
//! `events`, or `payments` that reach country via the landing row) plus the
//! country bind values via [`SegmentFilters::append_country_params`]. The other
//! (session-scoped) dimensions are appended together via
//! [`SegmentFilters::session_filter_clause`], which builds self-contained
//! `EXISTS` sub-queries keyed on the outer query's session id. Those clauses
//! inline their values as literals (validated `i64` ids, or text quoted by
//! [`sql_text_literal`]), so they add no bind parameters and never disturb the
//! country placeholder numbering.

use rusqlite::types::ToSql;
use serde::Deserialize;

use crate::referrer::{ReferrerType, SourceGroup, source_group_for_landing};
use crate::util::decode_query_component;

/// A revenue-source segment value. A session's source is an attribute of its
/// landing row: a campaign (`campaigns.id`, regardless of referrer), a plain
/// referrer (`referrers.id` with no campaign), or the direct bucket (no
/// campaign and no referrer). Mirrors `revenue_source::BucketKey`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SourceFilter {
    Direct,
    Referrer(i64),
    Campaign(i64),
}

impl SourceFilter {
    /// The source a landing row belongs to, given its live (not soft-deleted)
    /// campaign and its referrer. [`SegmentFilters::source_predicate`] also
    /// matches a landing to a `campaign-<id>` filter when that campaign was
    /// deleted, so a filter set before the deletion keeps selecting its landings.
    pub(crate) fn for_landing(live_campaign: Option<i64>, referrer: Option<i64>) -> Self {
        match (live_campaign, referrer) {
            (Some(campaign), _) => Self::Campaign(campaign),
            (None, Some(referrer)) => Self::Referrer(referrer),
            (None, None) => Self::Direct,
        }
    }

    /// The `source` query value; the inverse of [`parse_source_filter`].
    pub(crate) fn param(self) -> String {
        self.to_string()
    }
}

impl std::fmt::Display for SourceFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Direct => f.write_str("direct"),
            Self::Referrer(id) => write!(f, "referrer-{id}"),
            Self::Campaign(id) => write!(f, "campaign-{id}"),
        }
    }
}

/// A segment dimension read straight off the landing row. Each one has a
/// string value per landing ([`LandingAttribute::write_value`]) that is both the
/// filter's query value and the key of its overview breakdown row, and an SQL
/// predicate ([`LandingAttribute::sql_predicate`]) that must select exactly the
/// landings whose value matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum LandingAttribute {
    /// The marketing channel, a [`SourceGroup`] name.
    Channel,
    /// `{country}-{region}`, e.g. `US-NY`. Region codes are only unique within a country.
    Region,
    /// `{country}-{city}`, e.g. `US-Springfield`.
    City,
    /// `landings.browser_family`, `unknown` when NULL.
    Browser,
    /// `landings.os`, `unknown` when NULL.
    Os,
    /// `landings.device`.
    Device,
}

/// The landing columns [`LandingAttribute::write_value`] reads.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct LandingFacts<'a> {
    pub(crate) country: Option<&'a str>,
    pub(crate) region: Option<&'a str>,
    pub(crate) city: Option<&'a str>,
    pub(crate) browser_family: Option<&'a str>,
    pub(crate) os: Option<&'a str>,
    pub(crate) device: &'a str,
    pub(crate) has_referrer: bool,
    pub(crate) has_live_campaign: bool,
    pub(crate) referrer_type: Option<&'a str>,
}

/// Longest value a text attribute filter accepts, comfortably above any city name.
const MAX_ATTRIBUTE_VALUE_LEN: usize = 200;

impl LandingAttribute {
    pub(crate) const ALL: [Self; 6] = [
        Self::Channel,
        Self::Region,
        Self::City,
        Self::Browser,
        Self::Os,
        Self::Device,
    ];

    /// The query-string key and request-body `dimension` name.
    pub(crate) fn param(self) -> &'static str {
        match self {
            Self::Channel => "channel",
            Self::Region => "region",
            Self::City => "city",
            Self::Browser => "browser",
            Self::Os => "os",
            Self::Device => "device",
        }
    }

    pub(crate) fn from_param(key: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|attribute| attribute.param() == key)
    }

    /// This landing's value, or `None` when it has none (no resolved region or city).
    #[cfg(test)]
    pub(crate) fn value_of(self, facts: &LandingFacts<'_>) -> Option<String> {
        let mut value = String::new();
        self.write_value(facts, &mut value).then_some(value)
    }

    /// This landing's value, written into a reused buffer, which is cleared first.
    /// Returns false when the landing has no value.
    pub(crate) fn write_value(self, facts: &LandingFacts<'_>, out: &mut String) -> bool {
        out.clear();
        let place = |part: Option<&str>, out: &mut String| match (facts.country, part) {
            (Some(country), Some(part)) => {
                out.push_str(country);
                out.push('-');
                out.push_str(part);
                true
            }
            _ => false,
        };
        match self {
            Self::Channel => {
                out.push_str(
                    source_group_for_landing(
                        facts.has_live_campaign,
                        facts.has_referrer,
                        facts.referrer_type.and_then(ReferrerType::from_stored),
                    )
                    .as_str(),
                );
                true
            }
            Self::Region => place(facts.region, out),
            Self::City => place(facts.city, out),
            Self::Browser => {
                out.push_str(facts.browser_family.unwrap_or("unknown"));
                true
            }
            Self::Os => {
                out.push_str(facts.os.unwrap_or("unknown"));
                true
            }
            Self::Device => {
                out.push_str(facts.device);
                true
            }
        }
    }

    /// Whether a raw (already percent-decoded) value is filterable: within the
    /// length limit, free of control characters, and for channel, region and
    /// city, of the right shape. Browser, OS and device values are free text.
    pub(crate) fn accepts(self, value: &str) -> bool {
        if value.is_empty()
            || value.len() > MAX_ATTRIBUTE_VALUE_LEN
            || value.chars().any(char::is_control)
        {
            return false;
        }
        match self {
            Self::Channel => SourceGroup::parse(value).is_some(),
            Self::Region | Self::City => split_country_prefixed(value).is_some(),
            Self::Browser | Self::Os | Self::Device => true,
        }
    }

    /// SQL matching landings (aliased `alias`) whose [`Self::write_value`] is
    /// `value`. `value` must have passed [`Self::accepts`].
    fn sql_predicate(self, alias: &str, value: &str) -> String {
        match self {
            Self::Channel => {
                let group = SourceGroup::parse(value).expect("channel filter values are validated");
                channel_predicate(alias, group)
            }
            Self::Region | Self::City => {
                let (country, rest) =
                    split_country_prefixed(value).expect("place filter values are validated");
                let column = if self == Self::Region {
                    "region"
                } else {
                    "city"
                };
                format!(
                    "({alias}.country = {} AND {alias}.{column} = {})",
                    sql_text_literal(country),
                    sql_text_literal(rest)
                )
            }
            Self::Browser => format!(
                "COALESCE({alias}.browser_family, 'unknown') = {}",
                sql_text_literal(value)
            ),
            Self::Os => format!(
                "COALESCE({alias}.os, 'unknown') = {}",
                sql_text_literal(value)
            ),
            Self::Device => format!("{alias}.device = {}", sql_text_literal(value)),
        }
    }
}

/// A landing attribute filter as a request body carries it, e.g.
/// `{"dimension": "os", "value": "macos"}`.
#[derive(Debug, Deserialize)]
pub(crate) struct LandingAttributeParam {
    pub(crate) dimension: String,
    pub(crate) value: String,
}

/// Splits `US-New York` into (`US`, `New York`). The country is the two
/// uppercase letters `landings.country` stores, so the first dash is always the separator.
fn split_country_prefixed(value: &str) -> Option<(&str, &str)> {
    let (country, rest) = value.split_once('-')?;
    if country.len() != 2 || !country.bytes().all(|b| b.is_ascii_uppercase()) || rest.is_empty() {
        return None;
    }
    Some((country, rest))
}

/// SQL selecting the landings in `group`, mirroring [`source_group_for_landing`]:
/// a live campaign is always the campaigns channel, no referrer is direct, and
/// otherwise the stored `referrer_type` decides.
fn channel_predicate(alias: &str, group: SourceGroup) -> String {
    let campaign = format!("{alias}.campaign");
    let no_campaign = no_live_campaign_sql(&campaign);
    let referrer_type = format!("lower({alias}.referrer_type)");
    let referred = format!("{no_campaign} AND {alias}.referrer IS NOT NULL");
    match group {
        SourceGroup::Campaigns => format!(
            "(NOT {no_campaign} OR ({referred} AND {referrer_type} IN ('email', 'campaign', 'affiliate')))"
        ),
        SourceGroup::Direct => format!("({no_campaign} AND {alias}.referrer IS NULL)"),
        SourceGroup::Search => format!("({referred} AND {referrer_type} = 'search')"),
        SourceGroup::Social => format!("({referred} AND {referrer_type} = 'social')"),
        SourceGroup::Ai => format!("({referred} AND {referrer_type} = 'ai')"),
        SourceGroup::Referrers => format!(
            "({referred} AND ({alias}.referrer_type IS NULL OR {referrer_type} NOT IN \
             ('ai', 'search', 'social', 'email', 'campaign', 'affiliate')))"
        ),
    }
}

/// Quotes `value` as an SQLite string literal. SQLite has no backslash escapes,
/// so doubling `'` is the whole job; callers reject control characters, which
/// keeps a NUL from cutting the statement short.
fn sql_text_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// A session-scoped segment dimension. Used so a breakdown view can exempt
/// itself from its own filter (e.g. the revenue-sources view ignores the source
/// filter so users can add another source from it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SegmentDimension {
    Source,
    Page,
    LandingPage,
    Event,
    Landing(LandingAttribute),
}

/// The set of active global segment filters parsed from a request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SegmentFilters {
    /// Selected country codes (ISO 3166-1 alpha-2, uppercase). Empty means no
    /// country filter is applied.
    pub(crate) countries: Vec<String>,
    /// Selected revenue sources. Empty means no source filter is applied.
    pub(crate) sources: Vec<SourceFilter>,
    /// Selected page `paths.id`s. A session matches when it contains a pageview
    /// of any selected page. Empty means no page filter is applied.
    pub(crate) pages: Vec<i64>,
    /// Selected landing-page `paths.id`s. A session matches when its landing
    /// pageview is any selected page. Empty means no landing-page filter.
    pub(crate) landing_pages: Vec<i64>,
    /// Selected `event_names.id`s. A session matches when it contains an event
    /// with any selected name. Empty means no event filter is applied.
    pub(crate) events: Vec<i64>,
    /// Selected landing attribute values, validated by [`LandingAttribute::accepts`].
    pub(crate) attributes: Vec<(LandingAttribute, String)>,
}

impl SegmentFilters {
    /// True when no segment dimension has any selected value, i.e. this filter
    /// doesn't narrow anything.
    pub(crate) fn is_empty(&self) -> bool {
        self.countries.is_empty()
            && self.sources.is_empty()
            && self.pages.is_empty()
            && self.landing_pages.is_empty()
            && self.events.is_empty()
            && self.attributes.is_empty()
    }

    /// The selected values of one landing attribute.
    pub(crate) fn attribute_values(
        &self,
        attribute: LandingAttribute,
    ) -> impl Iterator<Item = &str> + '_ {
        self.attributes
            .iter()
            .filter(move |(selected, _)| *selected == attribute)
            .map(|(_, value)| value.as_str())
    }

    /// Whether a landing whose value for `attribute` is `value` passes that
    /// attribute's filter; always true when the attribute is unfiltered.
    pub(crate) fn attribute_matches(
        &self,
        attribute: LandingAttribute,
        value: Option<&str>,
    ) -> bool {
        let mut selected = self.attribute_values(attribute).peekable();
        selected.peek().is_none() || value.is_some_and(|value| selected.any(|s| s == value))
    }

    /// A copy without the given attributes' filters, for a query that applies
    /// them itself.
    pub(crate) fn without_attributes(&self, attributes: &[LandingAttribute]) -> Self {
        let mut copy = self.clone();
        copy.attributes
            .retain(|(attribute, _)| !attributes.contains(attribute));
        copy
    }

    /// Adds a landing attribute filter, ignoring invalid values and duplicates.
    pub(crate) fn add_attribute(&mut self, attribute: LandingAttribute, value: &str) {
        if attribute.accepts(value)
            && !self
                .attributes
                .iter()
                .any(|(selected, existing)| *selected == attribute && existing == value)
        {
            self.attributes.push((attribute, value.to_string()));
        }
    }

    /// Adds landing attribute filters from a request body, ignoring unknown
    /// dimensions and invalid values.
    pub(crate) fn set_attributes_from_params(&mut self, params: &[LandingAttributeParam]) {
        for param in params {
            if let Some(attribute) = LandingAttribute::from_param(&param.dimension) {
                self.add_attribute(attribute, &param.value);
            }
        }
    }

    /// Only the event filter, for queries that evaluate every other dimension themselves.
    pub(crate) fn events_only(&self) -> Self {
        Self {
            events: self.events.clone(),
            ..Self::default()
        }
    }

    /// Builds segment filters from a list of raw country codes (e.g. a request
    /// body). Codes are validated/normalized the same way as query params:
    /// two ASCII letters, uppercased, de-duplicated, invalid codes ignored.
    pub(crate) fn from_country_codes<I, S>(codes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut countries: Vec<String> = Vec::new();
        for code in codes {
            if let Some(code) = normalize_country_code(code.as_ref())
                && !countries.contains(&code)
            {
                countries.push(code);
            }
        }
        Self {
            countries,
            ..Self::default()
        }
    }

    /// Adds source filters parsed from a list of raw `source` strings (e.g. a
    /// request body). Values are parsed the same way as query params
    /// (`campaign-<id>`, `referrer-<id>`, `direct`); invalid values are ignored
    /// and duplicates are de-duplicated.
    pub(crate) fn set_sources_from_strings<I, S>(&mut self, values: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for value in values {
            if let Some(source) = parse_source_filter(value.as_ref())
                && !self.sources.contains(&source)
            {
                self.sources.push(source);
            }
        }
    }

    /// Adds page filters (`paths.id`) from a list of ids (e.g. a request body),
    /// de-duplicating against any already present.
    pub(crate) fn set_pages_from_ids(&mut self, ids: &[i64]) {
        for &id in ids {
            if !self.pages.contains(&id) {
                self.pages.push(id);
            }
        }
    }

    /// Adds landing-page filters (`paths.id`) from a list of ids (e.g. a request
    /// body), de-duplicating against any already present.
    pub(crate) fn set_landing_pages_from_ids(&mut self, ids: &[i64]) {
        for &id in ids {
            if !self.landing_pages.contains(&id) {
                self.landing_pages.push(id);
            }
        }
    }

    /// Adds event filters (`event_names.id`) from a list of ids (e.g. a request
    /// body), de-duplicating against any already present.
    pub(crate) fn set_events_from_ids(&mut self, ids: &[i64]) {
        for &id in ids {
            if !self.events.contains(&id) {
                self.events.push(id);
            }
        }
    }

    /// Builds ` AND <column> IN (?n, ?n+1, ...)` for queries that already scan
    /// `landings`, where `column` is the qualified country column (e.g.
    /// `"l.country"`). Placeholder numbering starts at `next_placeholder`.
    /// Returns an empty string when no country filter is set.
    pub(crate) fn country_in_clause(&self, column: &str, next_placeholder: usize) -> String {
        assert!(
            column.contains('.'),
            "country_in_clause column must be qualified (e.g. \"l.country\"), got {column:?}"
        );
        if self.countries.is_empty() {
            return String::new();
        }
        format!(
            " AND {column} IN ({})",
            placeholder_list(self.countries.len(), next_placeholder)
        )
    }

    /// Builds an `EXISTS` clause that reaches `landings.country` through the
    /// visitor's landing row, for queries on tables without a country column
    /// (`pageviews`, `events`, `payments`). `visitor_ts_column` is the qualified
    /// `visitor_timestamp_id` column of the outer query (e.g.
    /// `"pv.visitor_timestamp_id"`). Placeholder numbering starts at
    /// `next_placeholder`. Returns an empty string when no country filter is set.
    pub(crate) fn country_exists_clause(
        &self,
        visitor_ts_column: &str,
        next_placeholder: usize,
    ) -> String {
        assert!(
            visitor_ts_column.contains('.'),
            "country_exists_clause column must be qualified (e.g. \"pv.visitor_timestamp_id\"), got {visitor_ts_column:?}"
        );
        if self.countries.is_empty() {
            return String::new();
        }
        format!(
            " AND EXISTS (SELECT 1 FROM landings seg_l \
             WHERE seg_l.visitor_timestamp_id = {visitor_ts_column} \
             AND seg_l.country IN ({}))",
            placeholder_list(self.countries.len(), next_placeholder)
        )
    }

    /// Appends the country bind values in the same order used by the placeholder
    /// list, so they line up with the clause produced above.
    pub(crate) fn append_country_params<'a>(&'a self, params: &mut Vec<&'a dyn ToSql>) {
        for country in &self.countries {
            params.push(country);
        }
    }

    /// Builds the combined `AND EXISTS (...)` clause for every session-scoped
    /// dimension (source, page, landing page, event and the landing
    /// attributes), keyed on the outer query's session id
    /// column `session_ts_column` (a qualified `visitor_timestamp_id`, e.g.
    /// `"l.visitor_timestamp_id"` or `"pv.visitor_timestamp_id"`).
    ///
    /// `exclude` names a dimension the current view should not filter on (its
    /// own breakdown), matching the "a filter never narrows its own view" rule.
    ///
    /// The returned fragment inlines its values as SQL literals (validated `i64`
    /// ids, and attribute text quoted by [`sql_text_literal`]), so it
    /// carries no bind parameters and can be appended after the country clause
    /// without renumbering placeholders. Returns an empty string when no
    /// applicable session-scoped filter is active.
    pub(crate) fn session_filter_clause(
        &self,
        session_ts_column: &str,
        exclude: Option<SegmentDimension>,
    ) -> String {
        assert!(
            session_ts_column.contains('.'),
            "session_filter_clause column must be qualified (e.g. \"l.visitor_timestamp_id\"), \
             got {session_ts_column:?}; an unqualified column self-correlates against this \
             clause's own EXISTS subqueries instead of the outer query's row"
        );
        let mut clause = String::new();
        if exclude != Some(SegmentDimension::Source)
            && let Some(predicate) = self.source_predicate("seg_src")
        {
            clause.push_str(&format!(
                " AND EXISTS (SELECT 1 FROM landings seg_src \
                     WHERE seg_src.visitor_timestamp_id = {session_ts_column} AND {predicate})"
            ));
        }
        if exclude != Some(SegmentDimension::Page) && !self.pages.is_empty() {
            clause.push_str(&format!(
                " AND EXISTS (SELECT 1 FROM pageviews seg_pv \
                 WHERE seg_pv.visitor_timestamp_id = {session_ts_column} \
                 AND seg_pv.path_id IN ({}))",
                id_list(&self.pages)
            ));
        }
        if exclude != Some(SegmentDimension::LandingPage) && !self.landing_pages.is_empty() {
            // The landing pageview's timestamp_id equals the session id.
            clause.push_str(&format!(
                " AND EXISTS (SELECT 1 FROM pageviews seg_lp \
                 WHERE seg_lp.timestamp_id = {session_ts_column} \
                 AND seg_lp.path_id IN ({}))",
                id_list(&self.landing_pages)
            ));
        }
        if exclude != Some(SegmentDimension::Event) && !self.events.is_empty() {
            // Events reach the session through their pageview's visitor id.
            clause.push_str(&format!(
                " AND EXISTS (SELECT 1 FROM events seg_ev \
                 JOIN pageviews seg_ev_pv ON seg_ev_pv.timestamp_id = seg_ev.page_timestamp_id \
                 WHERE seg_ev_pv.visitor_timestamp_id = {session_ts_column} \
                 AND seg_ev.event_name_id IN ({}))",
                id_list(&self.events)
            ));
        }
        let attribute_groups: Vec<String> = LandingAttribute::ALL
            .into_iter()
            .filter(|attribute| exclude != Some(SegmentDimension::Landing(*attribute)))
            .filter_map(|attribute| {
                let alternatives: Vec<String> = self
                    .attribute_values(attribute)
                    .map(|value| attribute.sql_predicate("seg_attr", value))
                    .collect();
                (!alternatives.is_empty()).then(|| format!("({})", alternatives.join(" OR ")))
            })
            .collect();
        if !attribute_groups.is_empty() {
            clause.push_str(&format!(
                " AND EXISTS (SELECT 1 FROM landings seg_attr \
                 WHERE seg_attr.visitor_timestamp_id = {session_ts_column} AND {})",
                attribute_groups.join(" AND ")
            ));
        }
        clause
    }

    /// Builds the `OR`-combined predicate matching any selected source, over the
    /// landings row aliased as `alias`. Campaign rows match on `campaign`
    /// regardless of referrer; referrer rows require no campaign; the direct
    /// bucket requires neither. Returns `None` when no source filter is active.
    fn source_predicate(&self, alias: &str) -> Option<String> {
        if self.sources.is_empty() {
            return None;
        }
        let mut campaigns: Vec<i64> = Vec::new();
        let mut referrers: Vec<i64> = Vec::new();
        let mut has_direct = false;
        for source in &self.sources {
            match source {
                SourceFilter::Campaign(id) => campaigns.push(*id),
                SourceFilter::Referrer(id) => referrers.push(*id),
                SourceFilter::Direct => has_direct = true,
            }
        }
        let mut alternatives: Vec<String> = Vec::new();
        if !campaigns.is_empty() {
            alternatives.push(format!("{alias}.campaign IN ({})", id_list(&campaigns)));
        }
        let no_campaign = no_live_campaign_sql(&format!("{alias}.campaign"));
        if !referrers.is_empty() {
            alternatives.push(format!(
                "({no_campaign} AND {alias}.referrer IN ({}))",
                id_list(&referrers)
            ));
        }
        if has_direct {
            alternatives.push(format!("({no_campaign} AND {alias}.referrer IS NULL)"));
        }
        if alternatives.is_empty() {
            return None;
        }
        Some(format!("({})", alternatives.join(" OR ")))
    }
}

/// Predicate for a landing `column` with no campaign, or only a deleted one.
/// Deleted campaigns drop out of the sources view, so their landings rank under
/// their referrer or as direct and filters on those buckets must include them.
pub(crate) fn no_live_campaign_sql(column: &str) -> String {
    format!(
        "({column} IS NULL OR {column} IN (SELECT id FROM campaigns WHERE deleted_at IS NOT NULL))"
    )
}

/// Formats a slice of ids as a comma-separated SQL literal list (`1, 2, 3`).
/// Safe to inline because the values are `i64`.
fn id_list(ids: &[i64]) -> String {
    ids.iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Parses a single `source` segment value: `campaign-<id>`, `referrer-<id>`, or
/// `direct`. Returns `None` for anything else (including non-numeric ids).
///
/// A dash delimiter (rather than a colon) keeps the value free of characters
/// that `URLSearchParams` percent-encodes, so query parsing needs no decoding.
fn parse_source_filter(value: &str) -> Option<SourceFilter> {
    if value == "direct" {
        return Some(SourceFilter::Direct);
    }
    if let Some(id) = value.strip_prefix("campaign-") {
        return id.parse::<i64>().ok().map(SourceFilter::Campaign);
    }
    if let Some(id) = value.strip_prefix("referrer-") {
        return id.parse::<i64>().ok().map(SourceFilter::Referrer);
    }
    None
}

/// Builds a comma-separated placeholder list like `?5, ?6, ?7`.
fn placeholder_list(count: usize, next_placeholder: usize) -> String {
    (0..count)
        .map(|offset| format!("?{}", next_placeholder + offset))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Parses global segment filters from a request target (path + query string).
///
/// Recognizes repeated `country=XX` parameters. Codes are validated as two
/// ASCII letters and uppercased; invalid codes and the sentinel `unknown` are
/// ignored. Duplicates are de-duplicated while preserving first-seen order.
pub(crate) fn parse_segment_filters(target: &str) -> SegmentFilters {
    let mut countries: Vec<String> = Vec::new();
    let mut sources: Vec<SourceFilter> = Vec::new();
    let mut pages: Vec<i64> = Vec::new();
    let mut landing_pages: Vec<i64> = Vec::new();
    let mut events: Vec<i64> = Vec::new();
    let mut with_attributes = SegmentFilters::default();
    for (key, value) in crate::query_params::query_pairs(target) {
        match key {
            "country" => {
                if let Some(code) = normalize_country_code(value)
                    && !countries.contains(&code)
                {
                    countries.push(code);
                }
            }
            "source" => {
                if let Some(source) = parse_source_filter(value)
                    && !sources.contains(&source)
                {
                    sources.push(source);
                }
            }
            "page" => {
                if let Ok(id) = value.parse::<i64>()
                    && !pages.contains(&id)
                {
                    pages.push(id);
                }
            }
            "landing_page" => {
                if let Ok(id) = value.parse::<i64>()
                    && !landing_pages.contains(&id)
                {
                    landing_pages.push(id);
                }
            }
            "event" => {
                if let Ok(id) = value.parse::<i64>()
                    && !events.contains(&id)
                {
                    events.push(id);
                }
            }
            key => {
                if let Some(attribute) = LandingAttribute::from_param(key)
                    && let Some(value) = decode_query_component(value)
                {
                    with_attributes.add_attribute(attribute, &value);
                }
            }
        }
    }
    SegmentFilters {
        countries,
        sources,
        pages,
        landing_pages,
        events,
        attributes: with_attributes.attributes,
    }
}

/// Validates and normalizes a country code to uppercase ISO alpha-2, returning
/// `None` for anything that isn't exactly two ASCII letters.
fn normalize_country_code(value: &str) -> Option<String> {
    if value.len() != 2 || !value.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    Some(value.to_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_repeated_country_params() {
        let filters = parse_segment_filters(
            "/v2/read/projects/1/audience?start=1&end=2&country=US&country=ca",
        );
        assert_eq!(filters.countries, vec!["US".to_string(), "CA".to_string()]);
    }

    #[test]
    fn dedupes_and_ignores_invalid_codes() {
        let filters = parse_segment_filters(
            "?country=US&country=us&country=USA&country=1&country=unknown&country=GB",
        );
        assert_eq!(filters.countries, vec!["US".to_string(), "GB".to_string()]);
    }

    #[test]
    fn empty_when_no_country_param() {
        let filters = parse_segment_filters("/v2/read/projects/1/audience?start=1&end=2");
        assert!(filters.countries.is_empty());
        assert_eq!(filters.country_in_clause("l.country", 3), "");
        assert_eq!(
            filters.country_exists_clause("pv.visitor_timestamp_id", 3),
            ""
        );
    }

    #[test]
    fn builds_in_clause_with_placeholder_offset() {
        let filters = SegmentFilters {
            countries: vec!["US".to_string(), "CA".to_string()],
            sources: Vec::new(),
            pages: Vec::new(),
            landing_pages: Vec::new(),
            events: Vec::new(),
            ..Default::default()
        };
        assert_eq!(
            filters.country_in_clause("l.country", 3),
            " AND l.country IN (?3, ?4)"
        );
    }

    #[test]
    fn builds_exists_clause_with_placeholder_offset() {
        let filters = SegmentFilters {
            countries: vec!["US".to_string()],
            sources: Vec::new(),
            pages: Vec::new(),
            landing_pages: Vec::new(),
            events: Vec::new(),
            ..Default::default()
        };
        assert_eq!(
            filters.country_exists_clause("pv.visitor_timestamp_id", 4),
            " AND EXISTS (SELECT 1 FROM landings seg_l \
             WHERE seg_l.visitor_timestamp_id = pv.visitor_timestamp_id \
             AND seg_l.country IN (?4))"
        );
    }

    #[test]
    fn parses_repeated_source_params() {
        let filters = parse_segment_filters(
            "?source=campaign-12&source=referrer-7&source=direct&source=campaign-12",
        );
        assert_eq!(
            filters.sources,
            vec![
                SourceFilter::Campaign(12),
                SourceFilter::Referrer(7),
                SourceFilter::Direct,
            ]
        );
    }

    #[test]
    fn ignores_invalid_source_params() {
        let filters =
            parse_segment_filters("?source=campaign-&source=campaign-x&source=bogus&source=12");
        assert!(filters.sources.is_empty());
    }

    #[test]
    fn source_session_clause_is_empty_without_filter() {
        let filters = SegmentFilters::default();
        assert_eq!(
            filters.session_filter_clause("l.visitor_timestamp_id", None),
            ""
        );
    }

    #[test]
    fn source_session_clause_combines_dimensions_with_or() {
        let filters = SegmentFilters {
            countries: Vec::new(),
            sources: vec![
                SourceFilter::Campaign(3),
                SourceFilter::Campaign(4),
                SourceFilter::Referrer(9),
                SourceFilter::Direct,
            ],
            pages: Vec::new(),
            landing_pages: Vec::new(),
            events: Vec::new(),
            ..Default::default()
        };
        assert_eq!(
            filters.session_filter_clause("l.visitor_timestamp_id", None),
            " AND EXISTS (SELECT 1 FROM landings seg_src \
             WHERE seg_src.visitor_timestamp_id = l.visitor_timestamp_id \
             AND (seg_src.campaign IN (3, 4) OR \
             ((seg_src.campaign IS NULL OR seg_src.campaign IN \
             (SELECT id FROM campaigns WHERE deleted_at IS NOT NULL)) AND seg_src.referrer IN (9)) OR \
             ((seg_src.campaign IS NULL OR seg_src.campaign IN \
             (SELECT id FROM campaigns WHERE deleted_at IS NOT NULL)) AND seg_src.referrer IS NULL)))"
        );
    }

    #[test]
    fn source_session_clause_respects_own_view_exclusion() {
        let filters = SegmentFilters {
            countries: Vec::new(),
            sources: vec![SourceFilter::Direct],
            pages: Vec::new(),
            landing_pages: Vec::new(),
            events: Vec::new(),
            ..Default::default()
        };
        assert_eq!(
            filters.session_filter_clause("l.visitor_timestamp_id", Some(SegmentDimension::Source)),
            ""
        );
    }

    #[test]
    fn page_session_clause_uses_in_list_and_respects_exclusion() {
        let filters = SegmentFilters {
            countries: Vec::new(),
            sources: Vec::new(),
            pages: vec![5, 8],
            landing_pages: Vec::new(),
            events: Vec::new(),
            ..Default::default()
        };
        assert_eq!(
            filters.session_filter_clause("pv.visitor_timestamp_id", None),
            " AND EXISTS (SELECT 1 FROM pageviews seg_pv \
             WHERE seg_pv.visitor_timestamp_id = pv.visitor_timestamp_id \
             AND seg_pv.path_id IN (5, 8))"
        );
        assert_eq!(
            filters.session_filter_clause("pv.visitor_timestamp_id", Some(SegmentDimension::Page)),
            ""
        );
    }

    #[test]
    fn parses_repeated_page_params() {
        let filters = parse_segment_filters("?page=5&page=8&page=5&page=abc");
        assert_eq!(filters.pages, vec![5, 8]);
    }

    #[test]
    fn landing_page_session_clause_matches_landing_pageview() {
        let filters = SegmentFilters {
            countries: Vec::new(),
            sources: Vec::new(),
            pages: Vec::new(),
            landing_pages: vec![3, 7],
            events: Vec::new(),
            ..Default::default()
        };
        assert_eq!(
            filters.session_filter_clause("l.visitor_timestamp_id", None),
            " AND EXISTS (SELECT 1 FROM pageviews seg_lp \
             WHERE seg_lp.timestamp_id = l.visitor_timestamp_id \
             AND seg_lp.path_id IN (3, 7))"
        );
        assert_eq!(
            filters.session_filter_clause(
                "l.visitor_timestamp_id",
                Some(SegmentDimension::LandingPage)
            ),
            ""
        );
    }

    #[test]
    fn parses_repeated_landing_page_params() {
        let filters = parse_segment_filters("?landing_page=3&landing_page=7&landing_page=3");
        assert_eq!(filters.landing_pages, vec![3, 7]);
    }

    #[test]
    fn event_session_clause_matches_event_through_pageview_and_respects_exclusion() {
        let filters = SegmentFilters {
            countries: Vec::new(),
            sources: Vec::new(),
            pages: Vec::new(),
            landing_pages: Vec::new(),
            events: vec![4, 9],
            ..Default::default()
        };
        assert_eq!(
            filters.session_filter_clause("pv.visitor_timestamp_id", None),
            " AND EXISTS (SELECT 1 FROM events seg_ev \
             JOIN pageviews seg_ev_pv ON seg_ev_pv.timestamp_id = seg_ev.page_timestamp_id \
             WHERE seg_ev_pv.visitor_timestamp_id = pv.visitor_timestamp_id \
             AND seg_ev.event_name_id IN (4, 9))"
        );
        assert_eq!(
            filters.session_filter_clause("pv.visitor_timestamp_id", Some(SegmentDimension::Event)),
            ""
        );
    }

    #[test]
    fn parses_repeated_event_params() {
        let filters = parse_segment_filters("?event=4&event=9&event=4&event=abc");
        assert_eq!(filters.events, vec![4, 9]);
    }

    #[test]
    fn parses_landing_attribute_params() {
        let filters = parse_segment_filters(
            "?channel=search&channel=bogus&region=US-NY&region=us-ny&region=US-\
             &city=US-New+York&city=FR-Saint-%C3%89tienne&browser=chrome&os=macos\
             &device=mobile&device=mobile",
        );
        assert_eq!(
            filters.attributes,
            vec![
                (LandingAttribute::Channel, "search".to_string()),
                (LandingAttribute::Region, "US-NY".to_string()),
                (LandingAttribute::City, "US-New York".to_string()),
                (LandingAttribute::City, "FR-Saint-Étienne".to_string()),
                (LandingAttribute::Browser, "chrome".to_string()),
                (LandingAttribute::Os, "macos".to_string()),
                (LandingAttribute::Device, "mobile".to_string()),
            ]
        );
        assert!(!filters.is_empty());
    }

    #[test]
    fn rejects_attribute_values_with_control_characters() {
        let filters = parse_segment_filters("?city=US-a%00b&os=mac%0Aos&device=%ZZ");
        assert!(filters.attributes.is_empty());
    }

    #[test]
    fn attribute_filters_from_body_params() {
        let mut filters = SegmentFilters::default();
        filters.set_attributes_from_params(&[
            LandingAttributeParam {
                dimension: "os".to_string(),
                value: "ios".to_string(),
            },
            LandingAttributeParam {
                dimension: "planet".to_string(),
                value: "mars".to_string(),
            },
        ]);
        assert_eq!(
            filters.attributes,
            vec![(LandingAttribute::Os, "ios".to_string())]
        );
    }

    #[test]
    fn excluded_attribute_is_left_out_of_the_session_clause() {
        let filters = parse_segment_filters("?os=ios&device=mobile");
        let clause = filters.session_filter_clause(
            "l.visitor_timestamp_id",
            Some(SegmentDimension::Landing(LandingAttribute::Os)),
        );
        assert!(clause.contains("seg_attr.device = 'mobile'"), "{clause}");
        assert!(!clause.contains("ios"), "{clause}");
    }

    /// Every attribute's SQL predicate must select exactly the landings whose
    /// Rust [`LandingAttribute::value_of`] is the filter value, or a filter
    /// applied in SQL and the same filter applied by the overview breakdowns
    /// would disagree.
    #[test]
    fn attribute_sql_predicates_match_value_of() {
        use crate::test_utils::{LandingFixture, insert_landing};
        use fxhash::FxHashSet;

        let conn = crate::test_utils::open_migrated_db();
        conn.execute_batch(
            "INSERT INTO paths (id, path, hostname) VALUES (1, '/', 'acme.test');
             INSERT INTO referrers (id, referrer) VALUES (1, 'google.com'), (2, 'blog.test');
             INSERT INTO campaigns (id, campaign, first_seen_at) VALUES (1, 'live', 0);
             INSERT INTO campaigns (id, campaign, first_seen_at, deleted_at)
                 VALUES (2, 'gone', 0, 1);",
        )
        .expect("seed");

        let landings = [
            (None, None, None),
            (Some(1), None, Some("search")),
            (Some(1), None, Some("SEARCH")),
            (Some(2), None, Some("social")),
            (Some(2), None, Some("ai")),
            (Some(2), None, Some("email")),
            (Some(2), None, Some("affiliate")),
            (Some(2), None, Some("campaign")),
            (Some(2), None, Some("newsletter")),
            (Some(2), None, None),
            (None, Some(1), None),
            (Some(1), Some(1), Some("search")),
            (Some(1), Some(2), Some("search")),
            (None, Some(2), None),
            (None, None, Some("search")),
        ];
        let places = [
            (Some("US"), Some("NY"), Some("New York")),
            (Some("IT"), Some("65"), Some("L'Aquila")),
            (None, Some("NY"), Some("Nowhere")),
            (Some("US"), None, None),
        ];
        let tech = [
            (Some("chrome"), Some("macos"), "desktop"),
            (None, None, "mobile"),
            (Some("Safari"), Some("ios"), "tablet"),
        ];
        let mut facts_by_id = Vec::new();
        for (index, (referrer, campaign, referrer_type)) in landings.into_iter().enumerate() {
            let (country, region, city) = places[index % places.len()];
            let (browser_family, os, device) = tech[index % tech.len()];
            let visitor_timestamp_id = index as i64 + 1;
            insert_landing(
                &conn,
                LandingFixture {
                    visitor_timestamp_id,
                    path_id: 1,
                    country,
                    region,
                    city,
                    browser_family,
                    os,
                    device: Some(device),
                    referrer,
                    campaign,
                    referrer_type,
                },
            );
            let facts = LandingFacts {
                country,
                region,
                city,
                browser_family,
                os,
                device,
                has_referrer: referrer.is_some(),
                has_live_campaign: campaign == Some(1),
                referrer_type,
            };
            facts_by_id.push((visitor_timestamp_id, facts));
        }

        for attribute in LandingAttribute::ALL {
            let mut values: Vec<String> = facts_by_id
                .iter()
                .filter_map(|(_, facts)| attribute.value_of(facts))
                .collect();
            if attribute == LandingAttribute::Channel {
                values.extend(
                    SourceGroup::ALL
                        .iter()
                        .map(|group| group.as_str().to_string()),
                );
            }
            values.push("US-Nowhere".to_string());
            for value in values {
                let mut filters = SegmentFilters::default();
                filters.add_attribute(attribute, &value);
                if filters.attributes.is_empty() {
                    continue;
                }
                let clause = filters.session_filter_clause("l.visitor_timestamp_id", None);
                let from_sql: FxHashSet<i64> = conn
                    .with_query(
                        &format!(
                            "SELECT l.visitor_timestamp_id FROM landings l WHERE 1 = 1{clause}"
                        ),
                        [],
                        |mut rows| {
                            let mut ids = FxHashSet::default();
                            while let Some(row) = rows.next()? {
                                ids.insert(row.get(0)?);
                            }
                            Ok(ids)
                        },
                    )
                    .expect("filter query");
                let from_rust: FxHashSet<i64> = facts_by_id
                    .iter()
                    .filter(|(_, facts)| attribute.value_of(facts).as_deref() == Some(&value))
                    .map(|(id, _)| *id)
                    .collect();
                assert_eq!(from_sql, from_rust, "{attribute:?} = {value:?}");
            }
        }
    }
}
