//! The HTTP contract between the RevScope server and its clients: request and
//! response bodies, route paths, the integration catalog, and the tracker snippet.
//!
//! The server and the CLI both compile against these types, so a change to the
//! contract is a compile error in whichever side wasn't updated.
//!
//! A released CLI talks to servers newer than itself. So responses may gain fields
//! (serde ignores unknown ones), and every enum a response carries has an `Unknown`
//! variant. Removing or renaming a field breaks released clients.

pub mod auth;
pub mod errors;
pub mod ga_import;
pub mod install;
pub mod integrations;
pub mod oauth;
pub mod paths;
pub mod projects;
pub mod query;
pub mod settings;
pub mod troubleshooting;
