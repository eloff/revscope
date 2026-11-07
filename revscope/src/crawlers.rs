//! Known AI/search crawler tokens, operators, and categories.
//!
//! Tokens are matched as case-insensitive substrings of a lowercased user
//! agent. Longer / more specific tokens are listed first so `Claude-SearchBot`
//! wins over `ClaudeBot`.

use std::net::IpAddr;

use rusqlite::OptionalExtension;
use rusqlite::params;

use crate::ip_ranges;

/// `(match_token, cloudflare_token, operator, category)`
///
/// `match_token` is the lowercase substring matched against a lowercased
/// user agent. `cloudflare_token` is the case-sensitive needle for GraphQL
/// `userAgent_like`. Cloudflare's LIKE is case-sensitive: `%googlebot%`
/// matches nothing, `%Googlebot%` matches.
const CRAWLERS: &[(&str, &str, &str, &str)] = &[
    (
        "google-cloudvertexbot",
        "Google-CloudVertexBot",
        "google",
        "ai_crawler",
    ),
    (
        "claude-searchbot",
        "Claude-SearchBot",
        "anthropic",
        "ai_search",
    ),
    ("claude-user", "Claude-User", "anthropic", "ai_assistant"),
    ("claudebot", "ClaudeBot", "anthropic", "ai_crawler"),
    ("chatgpt-user", "ChatGPT-User", "openai", "ai_assistant"),
    ("oai-searchbot", "OAI-SearchBot", "openai", "ai_search"),
    ("gptbot", "GPTBot", "openai", "ai_crawler"),
    (
        "perplexity-user",
        "Perplexity-User",
        "perplexity",
        "ai_assistant",
    ),
    ("perplexitybot", "PerplexityBot", "perplexity", "ai_search"),
    (
        "cloudflarebrowserrenderingcrawler",
        "CloudflareBrowserRenderingCrawler",
        "cloudflare",
        "ai_browser",
    ),
    (
        "meta-externalfetcher",
        "meta-externalfetcher",
        "meta",
        "ai_assistant",
    ),
    (
        "meta-externalagent",
        "meta-externalagent",
        "meta",
        "ai_crawler",
    ),
    ("facebookbot", "FacebookBot", "meta", "ai_crawler"),
    (
        "arquivo-web-crawler",
        "Arquivo-web-crawler",
        "arquivo",
        "archive",
    ),
    (
        "archive.org_bot",
        "archive.org_bot",
        "archive.org",
        "archive",
    ),
    ("anchor browser", "Anchor Browser", "anchor", "ai_browser"),
    (
        "mistralai-user",
        "MistralAI-User",
        "mistral",
        "ai_assistant",
    ),
    (
        "duckassistbot",
        "DuckAssistBot",
        "duckduckgo",
        "ai_assistant",
    ),
    ("tiktokspider", "TikTokSpider", "bytedance", "ai_crawler"),
    ("bytespider", "Bytespider", "bytedance", "ai_crawler"),
    ("headlesschrome", "HeadlessChrome", "unknown", "headless"),
    ("proratainc", "ProRataInc", "prorata", "ai_crawler"),
    ("manus-user", "Manus-User", "manus", "ai_assistant"),
    ("terracotta", "Terracotta", "terracotta", "ai_browser"),
    ("novellum", "Novellum", "novellum", "ai_browser"),
    ("googlebot", "Googlebot", "google", "search_engine"),
    ("baiduspider", "Baiduspider", "baidu", "search_engine"),
    ("amazonbot", "Amazonbot", "amazon", "ai_crawler"),
    ("applebot", "Applebot", "apple", "ai_search"),
    ("petalbot", "PetalBot", "huawei", "search_engine"),
    ("timpibot", "TimpiBot", "timpi", "ai_crawler"),
    ("bingbot", "bingbot", "microsoft", "search_engine"),
    ("ccbot", "CCBot", "commoncrawl", "ai_crawler"),
];

/// Operator and category for a lowercased user-agent string.
pub fn classify(useragent: &str) -> Option<(&'static str, &'static str)> {
    CRAWLERS
        .iter()
        .find(|(token, _, _, _)| useragent.contains(token))
        .map(|(_, _, operator, category)| (*operator, *category))
}

/// True when the user-agent classifies as an operator and `ip` is in that
/// operator's published ranges.
pub fn verify_operator(useragent: &str, ip: Option<IpAddr>) -> bool {
    let Some(ip) = ip else {
        return false;
    };
    let Some((operator, _)) = classify(useragent) else {
        return false;
    };
    ip_ranges::operator_for(ip) == Some(operator)
}

/// Case-sensitive needles for Cloudflare GraphQL `userAgent_like: "%token%"`.
/// Headless Chrome is tracker-only; it is not in this list.
pub fn cloudflare_patterns() -> impl Iterator<Item = &'static str> {
    CRAWLERS
        .iter()
        .filter(|(token, _, _, category)| *token != "headlesschrome" && *category != "headless")
        .map(|(_, cloudflare_token, _, _)| *cloudflare_token)
}

/// Insert or reuse a `useragents` row, filling `operator`/`category` from
/// [`classify`] when the stored values are still null.
pub fn upsert_useragent(
    conn: &rusqlite::Connection,
    useragent: &str,
) -> Result<i64, rusqlite::Error> {
    let (operator, category) = match classify(useragent) {
        Some((operator, category)) => (Some(operator), Some(category)),
        None => (None, None),
    };
    // Written to work with or without the `ip` column and its index, so the
    // migration that drops them doesn't break this release while it drains.
    conn.execute(
        "INSERT INTO useragents (useragent, operator, category)
         SELECT ?1, ?2, ?3
         WHERE NOT EXISTS (SELECT 1 FROM useragents WHERE useragent = ?1)",
        params![useragent, operator, category],
    )?;
    conn.execute(
        "UPDATE useragents SET
            operator = COALESCE(operator, ?2),
            category = COALESCE(category, ?3)
         WHERE useragent = ?1",
        params![useragent, operator, category],
    )?;
    conn.query_row(
        "SELECT MIN(id) FROM useragents WHERE useragent = ?1 HAVING COUNT(*) > 0",
        params![useragent],
        |row| row.get::<_, i64>(0),
    )
    .optional()?
    .ok_or_else(|| rusqlite::Error::QueryReturnedNoRows)
}

/// Insert or reuse a `crawler_paths` row. Never writes to `paths`.
pub fn upsert_crawler_path(
    conn: &rusqlite::Connection,
    path: &str,
    hostname: &str,
) -> Result<i64, rusqlite::Error> {
    conn.execute(
        "INSERT INTO crawler_paths (path, hostname) VALUES (?1, ?2)
         ON CONFLICT(path, hostname) DO NOTHING",
        params![path, hostname],
    )?;
    conn.query_row(
        "SELECT id FROM crawler_paths WHERE path = ?1 AND hostname = ?2",
        params![path, hostname],
        |row| row.get::<_, i64>(0),
    )
    .optional()?
    .ok_or_else(|| rusqlite::Error::QueryReturnedNoRows)
}

/// Tracker (source=0): increment `estimated_count` on conflict.
pub fn upsert_tracker_crawler_view(
    conn: &rusqlite::Connection,
    hour: i64,
    useragent_id: i64,
    path_id: i64,
    estimated_count: i64,
    verified: bool,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO crawler_views_by_hour (
            hour, source, useragent_id, path_id, estimated_count, verified
         ) VALUES (?1, 0, ?2, ?3, ?4, ?5)
         ON CONFLICT(hour, source, useragent_id, path_id, verified) DO UPDATE SET
            estimated_count = crawler_views_by_hour.estimated_count + excluded.estimated_count",
        params![
            hour,
            useragent_id,
            path_id,
            estimated_count,
            i64::from(verified)
        ],
    )?;
    Ok(())
}

/// Cloudflare (source=1): replace when the fetched count is higher; never decrease.
/// Returns whether a stored value was kept because the fetch was lower.
pub fn upsert_cloudflare_crawler_view(
    conn: &rusqlite::Connection,
    hour: i64,
    useragent_id: i64,
    path_id: i64,
    estimated_count: i64,
) -> Result<CloudflareUpsert, rusqlite::Error> {
    let previous: Option<i64> = conn
        .query_row(
            "SELECT estimated_count FROM crawler_views_by_hour
             WHERE hour = ?1 AND source = 1 AND useragent_id = ?2 AND path_id = ?3
               AND verified = 1",
            params![hour, useragent_id, path_id],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(stored) = previous
        && estimated_count < stored
    {
        return Ok(CloudflareUpsert::KeptHigher {
            stored,
            fetched: estimated_count,
        });
    }
    let existed = previous.is_some();
    conn.execute(
        "INSERT INTO crawler_views_by_hour (
            hour, source, useragent_id, path_id, estimated_count, verified
         ) VALUES (?1, 1, ?2, ?3, ?4, 1)
         ON CONFLICT(hour, source, useragent_id, path_id, verified) DO UPDATE SET
            estimated_count = excluded.estimated_count
         WHERE excluded.estimated_count > crawler_views_by_hour.estimated_count",
        params![hour, useragent_id, path_id, estimated_count],
    )?;
    Ok(if existed {
        CloudflareUpsert::Updated
    } else {
        CloudflareUpsert::Inserted
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloudflareUpsert {
    Inserted,
    Updated,
    KeptHigher { stored: i64, fetched: i64 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_prefers_more_specific_tokens() {
        assert_eq!(
            classify("mozilla/5.0 compatible; claude-searchbot/1.0"),
            Some(("anthropic", "ai_search"))
        );
        assert_eq!(
            classify("mozilla/5.0 compatible; claudebot/1.0"),
            Some(("anthropic", "ai_crawler"))
        );
        assert_eq!(
            classify("mozilla/5.0 compatible; gptbot/1.4; +https://openai.com/gptbot"),
            Some(("openai", "ai_crawler"))
        );
        assert_eq!(classify("mozilla/5.0 chrome/120.0.0.0"), None);
    }

    #[test]
    fn cloudflare_tokens_are_canonical_case_of_match_tokens() {
        for (token, cloudflare_token, _, _) in CRAWLERS {
            assert_eq!(
                *token,
                cloudflare_token.to_ascii_lowercase(),
                "{cloudflare_token} must lower-case to {token}"
            );
        }
        let patterns: Vec<_> = cloudflare_patterns().collect();
        assert!(patterns.contains(&"Googlebot"));
        assert!(patterns.contains(&"GPTBot"));
        assert!(patterns.contains(&"bingbot"));
        assert!(!patterns.contains(&"googlebot"));
        assert!(!patterns.contains(&"gptbot"));
        assert!(!patterns.contains(&"HeadlessChrome"));
    }

    #[test]
    fn verify_operator_requires_matching_ip() {
        let googlebot = "mozilla/5.0 (compatible; googlebot/2.1; +http://www.google.com/bot.html)";
        let gptbot = "mozilla/5.0 (compatible; gptbot/1.4; +https://openai.com/gptbot)";
        let google_ip = "66.249.66.1".parse().unwrap();
        assert!(verify_operator(googlebot, Some(google_ip)));
        assert!(!verify_operator(gptbot, Some(google_ip)));
        assert!(!verify_operator(googlebot, None));
        assert!(!verify_operator(
            googlebot,
            Some("203.0.113.10".parse().unwrap())
        ));
    }

    fn create_useragents_table(conn: &rusqlite::Connection) {
        conn.execute_batch(
            "CREATE TABLE useragents (
                id INTEGER PRIMARY KEY,
                useragent TEXT NOT NULL,
                operator TEXT,
                category TEXT,
                CHECK (useragent == lower(useragent))
            );",
        )
        .unwrap();
    }

    #[test]
    fn upsert_useragent_reuses_the_row_and_fills_a_missing_operator() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        create_useragents_table(&conn);
        conn.execute(
            "INSERT INTO useragents (id, useragent) VALUES (7, 'gptbot/1.4')",
            [],
        )
        .unwrap();

        assert_eq!(upsert_useragent(&conn, "gptbot/1.4").unwrap(), 7);

        let operator: String = conn
            .query_row("SELECT operator FROM useragents WHERE id = 7", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(operator, "openai");
    }

    #[test]
    fn upsert_useragent_inserts_one_row_per_useragent() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        create_useragents_table(&conn);
        let first = upsert_useragent(&conn, "mystery").unwrap();
        let again = upsert_useragent(&conn, "mystery").unwrap();
        let other = upsert_useragent(&conn, "other").unwrap();
        assert_eq!(first, again);
        assert_ne!(first, other);
    }

    #[test]
    fn upsert_crawler_path_dedupes_on_path_and_hostname() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE crawler_paths (
                id INTEGER PRIMARY KEY,
                path TEXT NOT NULL,
                hostname TEXT NOT NULL,
                UNIQUE (path, hostname)
            );",
        )
        .unwrap();
        let first = upsert_crawler_path(&conn, "/robots.txt", "example.com").unwrap();
        let again = upsert_crawler_path(&conn, "/robots.txt", "example.com").unwrap();
        let other = upsert_crawler_path(&conn, "/robots.txt", "www.example.com").unwrap();
        assert_eq!(first, again);
        assert_ne!(first, other);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM crawler_paths", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2);
    }
}
