#[cfg(all(feature = "billing", feature = "self-hosted"))]
compile_error!("a self-hosted build has no billing; enable `billing` or `self-hosted`, not both");

mod account_subscription;
mod account_users;
mod acme;
mod audience;
mod auth;
mod authorize;
// A private repository checked out at src/billing; see its mod.rs. `cargo fmt`
// resolves modules whatever their cfg, so it would fail on a public checkout
// without the skip; the private repository formats itself.
#[cfg(feature = "billing")]
#[rustfmt::skip]
mod billing;
mod billing_guard;
mod campaign_spend;
mod campaigns;
mod chat;
mod chat_agent;
mod chat_config;
mod chat_store;
mod comparison;
mod connection_limit;
mod consent;
mod country_region;
mod crawler_detail;
mod crawler_report;
mod crawlers;
mod create_project;
mod crypto;
pub mod db;
mod delete_project;
mod download_db;
mod download_snapshots;
mod email;
mod events;
mod exclude_users;
mod funnels;
mod ga_import;
mod geoip;
#[cfg(not(feature = "self-hosted"))]
mod hosted_signup;
mod http;
mod id_token;
mod id_token_sign_in;
mod ingest;
mod install_snippet;
mod integration;
mod invite_user;
mod ip_ranges;
mod landing_labels;
mod landing_pages;
mod lru_cache;
mod main_db_cleanup;
mod mcp;
mod metrics;
mod overview;
mod page_revenue;
mod pages;
mod payment_activity;
mod payment_journey;
mod payments;
mod plan_retention;
mod projects;
mod query;
mod query_params;
mod query_token;
mod rate_limit;
mod referrer;
mod remove_user;
mod request_slots;
mod revenue_source;
mod robots;
mod routes;
mod schema;
mod search_terms;
mod segment_filter;
mod self_hosted;
mod sentry_report;
mod server;
pub mod server_context;
mod session_cache;
mod settings;
pub mod socket_cleanup;
mod subscription_status;
pub mod task;
#[doc(hidden)]
pub mod test_env;
#[cfg(test)]
mod test_utils;
mod timezone;
mod tls;
mod tokens;
mod transitions;
mod troubleshooting;
mod update_user;
mod user_access;
pub mod util;
mod uuid_text;
mod uuid_token;
mod visitors;
mod web_ui;

pub use acme::{AcmeConfig, prepare_acme, start_acme_task};
pub use billing_guard::require_billing_when_configured;
pub use connection_limit::raise_open_file_limit;
pub use country_region::major_cities_for_country;
pub use crawlers::{classify as classify_crawler_useragent, upsert_crawler_path, upsert_useragent};
pub use crypto::require_integration_encryption_key;
pub use download_snapshots::start_download_snapshot_cleanup_task;
pub use ga_import::start_ga_import_task;
pub use ingest::{
    ClickIdNetwork, LandingCampaignTuple, click_id_network_from_map, drain_ingest_on_shutdown,
    flush_all_ingest, merge_query_into_metadata, prepare_landing_attribution,
    prepare_landing_attribution_from_json, restore_landing_attribution_params_to_query,
    split_landing_query_params, split_landing_query_params_from_json,
};
pub use integration::start_integration_sync_task;
pub use ip_ranges::initialize as initialize_ip_ranges;
pub use main_db_cleanup::{run_main_db_cleanup, start_main_db_cleanup_task};
pub use referrer::{
    ReferrerType, SourceGroup, classify_referrer_type, referrer_is_google, referrer_is_search,
    source_group_for_landing,
};
pub use server::{
    ServerHooks, bind_listener, run_listener, run_server, run_server_with_hooks, run_tls_listener,
};
pub use tls::install_default_crypto_provider;
