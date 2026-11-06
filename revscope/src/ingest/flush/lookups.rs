//! Upserting the lookup rows (paths, referrers, user agents, event names,
//! campaigns) a batch refers to, caching their ids for the batch.

use compact_str::CompactString;
use fxhash::FxHashMap;
use rusqlite::OptionalExtension;
use rusqlite::params;

use crate::crawlers::upsert_useragent;
use crate::db::ProjectConnection;
use crate::ingest::attribution::LandingCampaignTuple;
use crate::ingest::error::PersistError;

pub(super) fn resolve_cached_path_id(
    conn: &ProjectConnection,
    cache: &mut FxHashMap<(CompactString, CompactString), i64>,
    path: &CompactString,
    hostname: &CompactString,
) -> Result<i64, PersistError> {
    let key = (path.clone(), hostname.clone());
    if let Some(&path_id) = cache.get(&key) {
        return Ok(path_id);
    }
    let path_id = resolve_path_id(conn, path, hostname)?
        .ok_or_else(|| PersistError::database("missing path id after upsert"))?;
    cache.insert(key, path_id);
    Ok(path_id)
}

pub(super) fn resolve_cached_referrer_id(
    conn: &ProjectConnection,
    cache: &mut FxHashMap<CompactString, i64>,
    referrer: &str,
) -> Result<i64, PersistError> {
    if let Some(&referrer_id) = cache.get(referrer) {
        return Ok(referrer_id);
    }
    let referrer_id = resolve_referrer_id(conn, referrer)?
        .ok_or_else(|| PersistError::database("missing referrer id after upsert"))?;
    cache.insert(CompactString::new(referrer), referrer_id);
    Ok(referrer_id)
}

pub(super) fn resolve_cached_useragent_id(
    conn: &ProjectConnection,
    cache: &mut FxHashMap<(CompactString, Option<CompactString>), i64>,
    useragent: &str,
    ip: Option<&str>,
) -> Result<i64, PersistError> {
    let cache_key = (CompactString::new(useragent), ip.map(CompactString::new));
    if let Some(&useragent_id) = cache.get(&cache_key) {
        return Ok(useragent_id);
    }
    let useragent_id = upsert_useragent(conn, useragent, ip)?;
    cache.insert(cache_key, useragent_id);
    Ok(useragent_id)
}

pub(super) fn resolve_cached_event_name_id(
    conn: &ProjectConnection,
    cache: &mut FxHashMap<CompactString, i64>,
    event_name: &CompactString,
) -> Result<i64, PersistError> {
    if let Some(&event_name_id) = cache.get(event_name) {
        return Ok(event_name_id);
    }
    let event_name_id = resolve_event_name_id(conn, event_name)?
        .ok_or_else(|| PersistError::database("missing event name id after upsert"))?;
    cache.insert(event_name.clone(), event_name_id);
    Ok(event_name_id)
}

pub(crate) fn resolve_path_id(
    conn: &ProjectConnection,
    value: &str,
    hostname: &str,
) -> Result<Option<i64>, rusqlite::Error> {
    conn.execute(
        "INSERT INTO paths (path, hostname) VALUES (?1, ?2) ON CONFLICT DO NOTHING",
        params![value, hostname],
    )?;
    conn.query_row(
        "SELECT id FROM paths WHERE path = ?1 AND hostname = ?2",
        params![value, hostname],
        |row| row.get::<_, i64>(0),
    )
    .optional()
}

pub(crate) fn resolve_referrer_id(
    conn: &ProjectConnection,
    value: &str,
) -> Result<Option<i64>, rusqlite::Error> {
    conn.execute(
        "INSERT INTO referrers (referrer) VALUES (?1) ON CONFLICT(referrer) DO NOTHING",
        params![value],
    )?;
    conn.query_row(
        "SELECT id FROM referrers WHERE referrer = ?1",
        params![value],
        |row| row.get::<_, i64>(0),
    )
    .optional()
}

pub(crate) fn resolve_landing_campaign_id(
    conn: &ProjectConnection,
    tuple: &LandingCampaignTuple,
    first_seen_at: i64,
) -> Result<Option<i64>, rusqlite::Error> {
    debug_assert!(tuple.has_campaign());
    conn.execute(
        &format!(
            "INSERT INTO campaigns (source, medium, campaign, network, first_seen_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             {}",
            crate::db::CAMPAIGNS_INSERT_ON_CONFLICT
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
        |row| row.get::<_, i64>(0),
    )
    .optional()
}
pub(crate) fn resolve_event_name_id(
    conn: &ProjectConnection,
    value: &str,
) -> Result<Option<i64>, rusqlite::Error> {
    conn.execute(
        "INSERT INTO event_names (name) VALUES (?1) ON CONFLICT(name) DO NOTHING",
        params![value],
    )?;
    conn.query_row(
        "SELECT id FROM event_names WHERE name = ?1",
        params![value],
        |row| row.get::<_, i64>(0),
    )
    .optional()
}
