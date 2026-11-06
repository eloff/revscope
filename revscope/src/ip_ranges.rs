//! Published crawler and Cloudflare CIDR feeds, compiled into the binary.
//!
//! Feeds are downloaded into `ipranges/` by `scripts/update-ip-ranges.mjs`, run by hand and committed.
//! Prefixes are parsed once into two sorted, disjoint interval lists (IPv4 and
//! IPv6). Lookup is the last interval whose start is `<=` the address; because
//! intervals do not overlap, that is the only candidate and the remainder of
//! the list can be ignored.

use std::net::IpAddr;
use std::sync::OnceLock;

use serde::Deserialize;

const FEEDS: &[(&str, &str)] = &[
    (
        "google",
        include_str!("../ipranges/google-common-crawlers.json"),
    ),
    (
        "google",
        include_str!("../ipranges/google-special-crawlers.json"),
    ),
    ("microsoft", include_str!("../ipranges/bingbot.json")),
    ("openai", include_str!("../ipranges/openai-gptbot.json")),
    ("openai", include_str!("../ipranges/openai-searchbot.json")),
    (
        "openai",
        include_str!("../ipranges/openai-chatgpt-user.json"),
    ),
    ("anthropic", include_str!("../ipranges/anthropic-bots.json")),
    ("perplexity", include_str!("../ipranges/perplexitybot.json")),
    (
        "perplexity",
        include_str!("../ipranges/perplexity-user.json"),
    ),
    ("duckduckgo", include_str!("../ipranges/duckassistbot.json")),
    ("commoncrawl", include_str!("../ipranges/ccbot.json")),
    ("apple", include_str!("../ipranges/applebot.json")),
    ("cloudflare", include_str!("../ipranges/cloudflare.json")),
];

static TABLE: OnceLock<IpRangeTable> = OnceLock::new();

#[derive(Debug)]
pub struct IpRangeTable {
    v4: Vec<AddrRange<u32>>,
    v6: Vec<AddrRange<u128>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AddrRange<T> {
    start: T,
    end: T,
    operator: &'static str,
}

#[derive(Deserialize)]
struct PrefixFeed {
    prefixes: Vec<Prefix>,
}

#[derive(Deserialize)]
struct Prefix {
    #[serde(rename = "ipv4Prefix")]
    ipv4_prefix: Option<String>,
    #[serde(rename = "ipv6Prefix")]
    ipv6_prefix: Option<String>,
}

#[derive(Clone, Copy)]
struct RawRange<T> {
    start: T,
    end: T,
    operator: &'static str,
}

/// Parse embedded feeds and panic only if they are corrupt. Call from `main`
/// so a bad checkout fails at boot rather than on the first ingest hit.
pub fn initialize() -> Result<(), String> {
    let table = table();
    if table.v4.is_empty() && table.v6.is_empty() {
        return Err("no crawler IP ranges loaded".to_string());
    }
    Ok(())
}

pub fn table() -> &'static IpRangeTable {
    TABLE.get_or_init(|| build_table().expect("embedded crawler IP ranges"))
}

pub fn operator_for(ip: IpAddr) -> Option<&'static str> {
    table().operator_for(ip)
}

pub fn canonical_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip),
        IpAddr::V4(_) => ip,
    }
}

impl IpRangeTable {
    pub fn operator_for(&self, ip: IpAddr) -> Option<&'static str> {
        match canonical_ip(ip) {
            IpAddr::V4(ip) => lookup(&self.v4, u32::from(ip)),
            IpAddr::V6(ip) => lookup(&self.v6, u128::from(ip)),
        }
    }
}

fn lookup<T: Copy + Ord>(ranges: &[AddrRange<T>], ip: T) -> Option<&'static str> {
    let index = ranges.partition_point(|range| range.start <= ip);
    let range = ranges.get(index.checked_sub(1)?)?;
    (ip <= range.end).then_some(range.operator)
}

fn build_table() -> Result<IpRangeTable, String> {
    parse_feeds(FEEDS)
}

fn parse_feeds(feeds: &[(&'static str, &str)]) -> Result<IpRangeTable, String> {
    let mut v4 = Vec::new();
    let mut v6 = Vec::new();
    for (operator, json) in feeds {
        let feed: PrefixFeed = serde_json::from_str(json)
            .map_err(|error| format!("invalid {operator} IP range JSON: {error}"))?;
        if feed.prefixes.is_empty() {
            return Err(format!("{operator} IP range JSON has no prefixes"));
        }
        for prefix in feed.prefixes {
            if let Some(cidr) = prefix.ipv4_prefix.as_deref() {
                let (start, end) = parse_ipv4_cidr(cidr)
                    .ok_or_else(|| format!("invalid IPv4 CIDR in {operator}: {cidr}"))?;
                v4.push(RawRange {
                    start,
                    end,
                    operator,
                });
            } else if let Some(cidr) = prefix.ipv6_prefix.as_deref() {
                let (start, end) = parse_ipv6_cidr(cidr)
                    .ok_or_else(|| format!("invalid IPv6 CIDR in {operator}: {cidr}"))?;
                v6.push(RawRange {
                    start,
                    end,
                    operator,
                });
            }
        }
    }
    Ok(IpRangeTable {
        v4: make_disjoint(v4),
        v6: make_disjoint(v6),
    })
}

fn parse_ipv4_cidr(cidr: &str) -> Option<(u32, u32)> {
    let (address, prefix) = split_cidr(cidr)?;
    let IpAddr::V4(address) = address.parse().ok()? else {
        return None;
    };
    if prefix > 32 {
        return None;
    }
    let ip = u32::from(address);
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    };
    let start = ip & mask;
    Some((start, start | !mask))
}

fn parse_ipv6_cidr(cidr: &str) -> Option<(u128, u128)> {
    let (address, prefix) = split_cidr(cidr)?;
    let IpAddr::V6(address) = address.parse().ok()? else {
        return None;
    };
    if prefix > 128 {
        return None;
    }
    let ip = u128::from(address);
    let mask = if prefix == 0 {
        0
    } else {
        u128::MAX << (128 - prefix)
    };
    let start = ip & mask;
    Some((start, start | !mask))
}

fn split_cidr(cidr: &str) -> Option<(&str, u8)> {
    let (address, prefix) = cidr.rsplit_once('/')?;
    Some((address, prefix.parse().ok()?))
}

fn make_disjoint<T: Addr>(mut ranges: Vec<RawRange<T>>) -> Vec<AddrRange<T>> {
    ranges.sort_by(|left, right| {
        span(left.start, left.end)
            .cmp(&span(right.start, right.end))
            .then_with(|| left.start.cmp(&right.start))
            .then_with(|| left.operator.cmp(right.operator))
    });
    let mut accepted = Vec::new();
    for range in ranges {
        insert_most_specific(&mut accepted, range);
    }
    accepted.sort_by_key(|left| left.start);
    merge_adjacent(&mut accepted);
    accepted
}

fn insert_most_specific<T: Addr>(accepted: &mut Vec<AddrRange<T>>, incoming: RawRange<T>) {
    let mut pieces = vec![(incoming.start, incoming.end)];
    for existing in accepted.iter() {
        let mut next = Vec::new();
        for (start, end) in pieces {
            if existing.end < start || existing.start > end {
                next.push((start, end));
                continue;
            }
            if start < existing.start {
                next.push((start, existing.start.saturating_sub_one()));
            }
            if end > existing.end {
                next.push((existing.end.saturating_add_one(), end));
            }
        }
        pieces = next;
        if pieces.is_empty() {
            break;
        }
    }
    for (start, end) in pieces {
        if start <= end {
            accepted.push(AddrRange {
                start,
                end,
                operator: incoming.operator,
            });
        }
    }
}

fn merge_adjacent<T: Addr>(ranges: &mut Vec<AddrRange<T>>) {
    let mut index = 0;
    while index + 1 < ranges.len() {
        let can_merge = {
            let left = &ranges[index];
            let right = &ranges[index + 1];
            left.operator == right.operator && left.end.saturating_add_one() == right.start
        };
        if can_merge {
            let right_end = ranges[index + 1].end;
            ranges[index].end = right_end;
            ranges.remove(index + 1);
        } else {
            index += 1;
        }
    }
}

fn span<T: Addr>(start: T, end: T) -> T {
    end.saturating_sub(start)
}

trait Addr: Copy + Ord + Eq {
    fn saturating_add_one(self) -> Self;
    fn saturating_sub_one(self) -> Self;
    fn saturating_sub(self, other: Self) -> Self;
}

impl Addr for u32 {
    fn saturating_add_one(self) -> Self {
        self.saturating_add(1)
    }
    fn saturating_sub_one(self) -> Self {
        self.saturating_sub(1)
    }
    fn saturating_sub(self, other: Self) -> Self {
        u32::saturating_sub(self, other)
    }
}

impl Addr for u128 {
    fn saturating_add_one(self) -> Self {
        self.saturating_add(1)
    }
    fn saturating_sub_one(self) -> Self {
        self.saturating_sub(1)
    }
    fn saturating_sub(self, other: Self) -> Self {
        u128::saturating_sub(self, other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn every_feed_parses_non_empty() {
        let table = table();
        assert!(!table.v4.is_empty());
        assert!(!table.v6.is_empty());
        assert!(
            table
                .v4
                .windows(2)
                .all(|pair| pair[0].start < pair[1].start)
        );
        assert!(
            table
                .v6
                .windows(2)
                .all(|pair| pair[0].start < pair[1].start)
        );
        assert!(table.v4.windows(2).all(|pair| pair[0].end < pair[1].start));
        assert!(table.v6.windows(2).all(|pair| pair[0].end < pair[1].start));
    }

    #[test]
    fn known_operator_ips() {
        assert_eq!(operator_for("66.249.66.1".parse().unwrap()), Some("google"));
        assert_eq!(
            operator_for("157.55.39.5".parse().unwrap()),
            Some("microsoft")
        );
        assert_eq!(
            operator_for("173.245.48.1".parse().unwrap()),
            Some("cloudflare")
        );
        assert_eq!(
            operator_for("2001:4860:4801:10::1".parse().unwrap()),
            Some("google")
        );
        assert_eq!(operator_for("10.0.0.1".parse().unwrap()), None);
        assert_eq!(operator_for("192.0.2.1".parse().unwrap()), None);
    }

    #[test]
    fn ipv4_mapped_looks_up_as_ipv4() {
        assert_eq!(
            operator_for("::ffff:66.249.66.1".parse().unwrap()),
            Some("google")
        );
        assert_eq!(
            operator_for("::ffff:173.245.48.1".parse().unwrap()),
            Some("cloudflare")
        );
    }

    #[test]
    fn nested_prefixes_prefer_the_most_specific_operator() {
        let json_outer = r#"{ "prefixes": [{ "ipv4Prefix": "10.0.0.0/8" }] }"#;
        let json_inner = r#"{ "prefixes": [{ "ipv4Prefix": "10.1.0.0/16" }] }"#;
        let table = parse_feeds(&[("outer", json_outer), ("inner", json_inner)]).expect("parse");
        assert_eq!(
            table.operator_for("10.1.2.3".parse().unwrap()),
            Some("inner")
        );
        assert_eq!(
            table.operator_for("10.2.0.1".parse().unwrap()),
            Some("outer")
        );
        assert_eq!(table.operator_for("11.0.0.1".parse().unwrap()), None);
    }

    #[test]
    fn same_operator_nesting_collapses() {
        let json = r#"{ "prefixes": [
            { "ipv4Prefix": "192.0.2.0/24" },
            { "ipv4Prefix": "192.0.2.0/25" }
        ] }"#;
        let table = parse_feeds(&[("google", json)]).expect("parse");
        assert_eq!(table.v4.len(), 1);
        assert_eq!(table.v4[0].start, u32::from(Ipv4Addr::new(192, 0, 2, 0)));
        assert_eq!(table.v4[0].end, u32::from(Ipv4Addr::new(192, 0, 2, 255)));
        assert_eq!(
            table.operator_for("192.0.2.1".parse().unwrap()),
            Some("google")
        );
    }

    #[test]
    fn parses_ipv6_cidr_boundaries() {
        let start = u128::from("2001:4860:4801:10::".parse::<Ipv6Addr>().unwrap());
        let (parsed_start, parsed_end) = parse_ipv6_cidr("2001:4860:4801:10::/64").unwrap();
        assert_eq!(parsed_start, start);
        assert_eq!(parsed_end, start | ((1u128 << 64) - 1));
    }
}
