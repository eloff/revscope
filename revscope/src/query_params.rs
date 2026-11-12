//! Query-string parameters shared by the report routes, and the sorting and
//! paging they control.

use std::borrow::Cow;
use std::cmp::Ordering;

use crate::comparison;
use crate::plan_retention::retained_range;

/// The `key=value` pairs of a request target's query string, as sent: values
/// are not percent-decoded, and a pair without `=` is skipped.
pub(crate) fn query_pairs(target: &str) -> impl Iterator<Item = (&str, &str)> {
    let query = target.split_once('?').map_or("", |(_path, query)| query);
    query.split('&').filter_map(|pair| pair.split_once('='))
}

/// The `start` and `end` parameters every ranged report shares. Unparseable
/// values read as absent.
pub(crate) struct DateRangeParams {
    pub(crate) start: Option<i64>,
    pub(crate) end: Option<i64>,
}

impl DateRangeParams {
    /// The required `[start, end)` range.
    pub(crate) fn validate(&self) -> Result<(i64, i64), &'static str> {
        comparison::validate_date_range(self.start, self.end)
    }
}

/// Parses a report's query string, reading `start` and `end` and handing each
/// other parameter to `other`. Empty values are skipped as absent, and a
/// repeated parameter resolves to its last value.
pub(crate) fn parse_report_query<'a>(
    target: &'a str,
    mut other: impl FnMut(&'a str, &'a str) -> Result<(), &'static str>,
) -> Result<DateRangeParams, &'static str> {
    let mut range = DateRangeParams {
        start: None,
        end: None,
    };
    for (key, value) in query_pairs(target).filter(|(_key, value)| !value.is_empty()) {
        match key {
            "start" => range.start = value.parse().ok(),
            "end" => range.end = value.parse().ok(),
            _ => other(key, value)?,
        }
    }
    Ok(range)
}

/// The required date range of a report with no parameters of its own.
pub(crate) fn parse_date_range_query(target: &str) -> Result<(i64, i64), &'static str> {
    parse_report_query(target, |_key, _value| Ok(()))?.validate()
}

/// `target` with its `start` moved up to `oldest`, so a report parsed from it
/// shows nothing older than the plan keeps. Every repeat of `start` is
/// replaced, since the parsers read the last. A range missing its bounds is
/// passed on as is for the report's own parser to refuse.
pub(crate) fn retained_target(
    target: &str,
    oldest: Option<i64>,
) -> Result<Cow<'_, str>, &'static str> {
    let range = parse_report_query(target, |_key, _value| Ok(()))?;
    let (Some(start), Some(end)) = (range.start, range.end) else {
        return Ok(Cow::Borrowed(target));
    };
    let (retained_start, _) = retained_range(start, end, oldest)?;
    if retained_start == start {
        return Ok(Cow::Borrowed(target));
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let mut pairs: Vec<String> = query
        .split('&')
        .filter(|pair| pair.split_once('=').is_none_or(|(key, _)| key != "start"))
        .filter(|pair| !pair.is_empty())
        .map(str::to_string)
        .collect();
    pairs.push(format!("start={retained_start}"));
    Ok(Cow::Owned(format!("{path}?{}", pairs.join("&"))))
}

/// A paged, sorted report's `offset`, `order_by` and `direction` parameters.
pub(crate) struct SortedPage<O> {
    pub(crate) offset: i64,
    pub(crate) order_by: O,
    pub(crate) direction: SortDirection,
    parse_order_by: fn(&str) -> Result<O, &'static str>,
}

impl<O> SortedPage<O> {
    /// The first page by `order_by`, descending, until [`Self::accept`] reads
    /// otherwise.
    pub(crate) fn new(order_by: O, parse_order_by: fn(&str) -> Result<O, &'static str>) -> Self {
        Self {
            offset: 0,
            order_by,
            direction: SortDirection::Desc,
            parse_order_by,
        }
    }

    /// Takes the parameter if it is one of the three, answering whether it was.
    pub(crate) fn accept(&mut self, key: &str, value: &str) -> Result<bool, &'static str> {
        match key {
            "offset" => self.offset = parse_offset(value)?,
            "order_by" => self.order_by = (self.parse_order_by)(value)?,
            "direction" => self.direction = SortDirection::parse(value)?,
            _ => return Ok(false),
        }
        Ok(true)
    }
}

/// Parses a row id parameter, which must be at least 1, answering `invalid`
/// for anything else.
pub(crate) fn parse_positive_id(value: &str, invalid: &'static str) -> Result<i64, &'static str> {
    value
        .parse::<i64>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or(invalid)
}

/// Sort direction for a report's `direction` parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SortDirection {
    Asc,
    Desc,
}

impl SortDirection {
    pub(crate) fn parse(value: &str) -> Result<Self, &'static str> {
        match value {
            "asc" => Ok(Self::Asc),
            "desc" => Ok(Self::Desc),
            _ => Err("direction must be asc or desc"),
        }
    }

    /// Orients an ascending comparison in this direction.
    pub(crate) fn apply(self, ascending: Ordering) -> Ordering {
        match self {
            Self::Asc => ascending,
            Self::Desc => ascending.reverse(),
        }
    }

    pub(crate) fn sql(self) -> &'static str {
        match self {
            Self::Asc => "ASC",
            Self::Desc => "DESC",
        }
    }
}

/// Parses a paginated report's `offset` parameter.
pub(crate) fn parse_offset(value: &str) -> Result<i64, &'static str> {
    let offset = value
        .parse::<i64>()
        .map_err(|_| "offset must be a number")?;
    if offset < 0 {
        return Err("offset must be at least 0");
    }
    Ok(offset)
}

/// One page of already sorted `rows` starting at `offset`, and the offset of
/// the next page when any rows remain after it.
pub(crate) fn paginate<T>(mut rows: Vec<T>, offset: i64, page_size: i64) -> (Vec<T>, Option<i64>) {
    let start = usize::try_from(offset).unwrap_or(0).min(rows.len());
    let end = start.saturating_add(page_size as usize).min(rows.len());
    let next_offset = (end < rows.len()).then_some(offset + page_size);
    rows.truncate(end);
    rows.drain(..start);
    (rows, next_offset)
}

/// Orders floats for sorting, with NaN greater than every number. NaN can't compare
/// equal to everything: that isn't a total order, and `sort_by` may panic on one.
pub(crate) fn cmp_f64(left: f64, right: f64) -> Ordering {
    left.partial_cmp(&right)
        .unwrap_or_else(|| left.is_nan().cmp(&right.is_nan()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats_sort_in_a_total_order_with_nan_last() {
        let mut values = [2.0, f64::NAN, -1.0, 0.0, f64::NAN, 1.5];
        values.sort_by(|left, right| cmp_f64(*left, *right));
        assert_eq!(values[..4], [-1.0, 0.0, 1.5, 2.0]);
        assert!(values[4].is_nan() && values[5].is_nan());
        assert_eq!(cmp_f64(-0.0, 0.0), Ordering::Equal);
    }

    #[test]
    fn report_query_reads_the_range_and_hands_on_the_rest() {
        let mut other = Vec::new();
        let range = parse_report_query("/x?start=10&end=&bucket=day&flag&end=20", |key, value| {
            other.push((key, value));
            Ok(())
        })
        .expect("parses");
        assert_eq!(range.validate(), Ok((10, 20)));
        assert_eq!(other, vec![("bucket", "day")]);
    }

    #[test]
    fn report_query_passes_on_errors_from_other_params() {
        assert_eq!(
            parse_report_query("/x?bad=1", |_key, _value| Err("bad")).err(),
            Some("bad")
        );
        assert_eq!(parse_date_range_query("/x?end=5"), Err("start is required"));
    }

    #[test]
    fn a_start_older_than_retention_moves_up_to_it() {
        assert_eq!(
            retained_target("/x?start=10&bucket=day&start=5&end=50", Some(20)).as_deref(),
            Ok("/x?bucket=day&end=50&start=20")
        );
        let unchanged = retained_target("/x?start=30&end=50", Some(20)).expect("in range");
        assert!(matches!(unchanged, Cow::Borrowed("/x?start=30&end=50")));
        assert!(matches!(
            retained_target("/x?start=10&end=50", None),
            Ok(Cow::Borrowed(_))
        ));
    }

    #[test]
    fn a_range_wholly_older_than_retention_is_refused() {
        assert_eq!(
            retained_target("/x?start=10&end=20", Some(20)),
            Err(crate::plan_retention::RANGE_OLDER_THAN_RETENTION)
        );
        // Left for the report's parser to name what's missing.
        assert!(matches!(
            retained_target("/x?end=20", Some(30)),
            Ok(Cow::Borrowed("/x?end=20"))
        ));
    }

    #[test]
    fn sorted_page_takes_only_its_own_params() {
        fn parse_order(value: &str) -> Result<u8, &'static str> {
            value.parse().map_err(|_| "bad order_by")
        }
        let mut sort = SortedPage::new(1, parse_order);
        assert_eq!(sort.accept("offset", "5"), Ok(true));
        assert_eq!(sort.accept("order_by", "3"), Ok(true));
        assert_eq!(sort.accept("direction", "asc"), Ok(true));
        assert_eq!(sort.accept("other", "x"), Ok(false));
        assert_eq!(sort.accept("order_by", "x"), Err("bad order_by"));
        assert_eq!(
            (sort.offset, sort.order_by, sort.direction),
            (5, 3, SortDirection::Asc)
        );
    }

    #[test]
    fn paginates_and_reports_the_next_offset_only_when_rows_remain() {
        let rows: Vec<i32> = (0..5).collect();
        assert_eq!(paginate(rows.clone(), 0, 2), (vec![0, 1], Some(2)));
        assert_eq!(paginate(rows.clone(), 2, 3), (vec![2, 3, 4], None));
        assert_eq!(paginate(rows.clone(), 4, 1), (vec![4], None));
        assert_eq!(paginate(rows, 9, 2), (vec![], None));
    }
}
