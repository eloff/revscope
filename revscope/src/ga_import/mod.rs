//! Importing history from Google Analytics 4. GA's API only returns daily
//! aggregates, so rows go in their own `imported_*` tables and reports that can
//! use them add them to what they count from tracked sessions.

mod api;
mod connect;
mod credential;
mod jobs;
mod list_imports;
mod read;
mod reports;
mod settings;
mod start_import;
mod stop_import;
mod store;
mod worker;

pub(crate) use api::GaEndpoints;
pub(crate) use connect::{ConnectRequest, GoogleOAuthClient, connect, oauth_url};
pub(crate) use jobs::{StrandedToken, revoke_stranded, take_project_tokens};
pub(crate) use list_imports::list_imports;
pub(crate) use read::{
    ImportedDays, ImportedEvent, ImportedPage, ImportedWindows, imported_coverage,
    parse_coverage_query,
};
pub(crate) use settings::{ImportedDataSettings, save_settings};
pub(crate) use start_import::{StartImportRequest, start_import};
pub(crate) use stop_import::{cancel_import, delete_import, parse_import_id};
pub use worker::start_ga_import_task;
