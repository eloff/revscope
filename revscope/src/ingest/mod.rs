//! Tracker ingest: the HTTP route, payload parsing and validation, enqueueing
//! hits into per-project batches, and flushing those batches to the project
//! database.
//!
//! A hit flows through the submodules in this order: [`route`] receives it,
//! [`parse`] decodes the [`wire`] format, [`enqueue`] (with [`pageview`] for
//! pageviews) resolves just enough to answer the tracker and queues rows on the
//! project's [`batch`], and [`flush`] writes the batch later. Rows the project
//! database would reject are caught by [`constraints`] and recorded via
//! [`failure`].

mod attribution;
mod batch;
mod constraints;
mod enqueue;
mod error;
mod failure;
mod flush;
mod lookup;
mod map;
mod normalize;
mod pageview;
mod parse;
mod route;
mod rows;
mod session;
#[cfg(test)]
mod test_support;
mod timestamp;
mod user_agent;
mod wire;

pub use attribution::{
    ClickIdNetwork, LandingCampaignTuple, click_id_network_from_map, merge_query_into_metadata,
    prepare_landing_attribution, prepare_landing_attribution_from_json,
    restore_landing_attribution_params_to_query, split_landing_query_params,
    split_landing_query_params_from_json,
};
pub(crate) use error::INGEST_PERSIST_INTERNAL_ERROR;
pub(crate) use failure::delete_expired_errors;
pub(crate) use flush::{
    FLUSH_INTERVAL_SECS, resolve_event_name_id, resolve_landing_campaign_id, resolve_path_id,
    resolve_referrer_id, start_ingest_flush_scheduler,
};
pub use flush::{drain_ingest_on_shutdown, flush_all_ingest};
pub(crate) use map::IngestMap;
pub(crate) use parse::extract_payload;
pub(crate) use route::handle_ingest_request;
