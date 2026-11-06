use std::sync::Arc;

use crate::download_db::DEMO_PROJECT_ID;
use crate::http::Response;
use crate::lru_cache::LruCache;
use crate::server_context::ServerContext;
use crate::timezone::BucketGranularity;

use super::{OverviewQuery, OverviewResponse};

/// Serialized demo overview JSON for unfiltered queries. The demo dashboard
/// mostly asks for one or two default ranges, so a couple of entries suffice.
pub(crate) type DemoOverviewResponseCache = LruCache<DemoOverviewCacheKey, Arc<[u8]>>;

pub(crate) const DEMO_OVERVIEW_CACHE_ENTRIES: usize = 2;

/// Everything an unfiltered demo overview response varies by, except the
/// project timezone; changing that clears the cache instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DemoOverviewCacheKey {
    start: i64,
    end: i64,
    bucket: BucketGranularity,
}

fn demo_overview_cache_key(
    project_id: &str,
    query: &OverviewQuery,
) -> Option<DemoOverviewCacheKey> {
    if project_id != DEMO_PROJECT_ID || !query.segments.is_empty() {
        return None;
    }
    Some(DemoOverviewCacheKey {
        start: query.start,
        end: query.end,
        bucket: query.bucket,
    })
}

/// Cache key when the query is cacheable and the overview cache is enabled.
fn cacheable_demo_overview_key(
    server_context: &ServerContext,
    project_id: &str,
    query: &OverviewQuery,
) -> Option<DemoOverviewCacheKey> {
    if server_context.overview_cache_disabled {
        return None;
    }
    demo_overview_cache_key(project_id, query)
}

/// Returns a cached demo overview JSON body when the query is cacheable and present.
pub(crate) fn take_cached_demo_overview(
    server_context: &ServerContext,
    project_id: &str,
    query: &OverviewQuery,
) -> Option<Vec<u8>> {
    let key = cacheable_demo_overview_key(server_context, project_id, query)?;
    server_context
        .demo_overview_cache
        .get(&key)
        .map(|body| body.to_vec())
}

/// Serialize an overview response, caching the JSON for cacheable demo queries.
pub(crate) fn respond_overview_json(
    server_context: &ServerContext,
    project_id: &str,
    query: &OverviewQuery,
    response: &OverviewResponse,
) -> Response {
    let body = serde_json::to_vec(response).unwrap_or_else(|_| b"{}".to_vec());
    if let Some(key) = cacheable_demo_overview_key(server_context, project_id, query) {
        server_context
            .demo_overview_cache
            .put(key, Arc::from(body.as_slice()));
    }
    Response::json("200 OK", body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::segment_filter::{SegmentFilters, parse_segment_filters};

    fn demo_query(segments: SegmentFilters) -> OverviewQuery {
        OverviewQuery {
            start: 10,
            end: 20,
            bucket: BucketGranularity::Daily,
            segments,
        }
    }

    #[test]
    fn only_applies_to_the_unfiltered_demo_project() {
        let context = ServerContext::new_in_memory("demo-overview-cache");
        let demo = demo_query(SegmentFilters::default());
        let filtered = demo_query(parse_segment_filters(
            "/v2/read/projects/1/overview?start=10&end=20&country=US",
        ));

        context.demo_overview_cache.put(
            demo_overview_cache_key(DEMO_PROJECT_ID, &demo).unwrap(),
            Arc::from(b"hit".as_slice()),
        );

        assert_eq!(
            take_cached_demo_overview(&context, DEMO_PROJECT_ID, &demo).as_deref(),
            Some(b"hit".as_slice())
        );
        assert!(take_cached_demo_overview(&context, DEMO_PROJECT_ID, &filtered).is_none());
        assert!(take_cached_demo_overview(&context, "2", &demo).is_none());
        assert!(demo_overview_cache_key(DEMO_PROJECT_ID, &filtered).is_none());
        assert!(demo_overview_cache_key("2", &demo).is_none());
    }

    #[test]
    fn disabling_the_cache_skips_hits() {
        let mut context = ServerContext::new_in_memory("demo-overview-cache-disabled");
        context.overview_cache_disabled = true;
        let demo = demo_query(SegmentFilters::default());
        context.demo_overview_cache.put(
            demo_overview_cache_key(DEMO_PROJECT_ID, &demo).unwrap(),
            Arc::from(b"hit".as_slice()),
        );

        assert!(take_cached_demo_overview(&context, DEMO_PROJECT_ID, &demo).is_none());
        assert!(cacheable_demo_overview_key(&context, DEMO_PROJECT_ID, &demo).is_none());
    }
}
