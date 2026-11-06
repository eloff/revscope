use std::net::IpAddr;
use std::sync::Arc;

use serde::Serialize;

use super::enqueue::{PersistOutcome, enqueue_items};
use super::error::{INGEST_PERSIST_INTERNAL_ERROR, PersistError};
use super::failure::{may_record_error, persist_error_to_main, persist_row_failure_to_main};
use super::parse::{ParsedPayloadOutcome, parse_and_validate_payload};
use super::session::session_cookie_headers;
use super::wire::HitKind;
use crate::exclude_users;
use crate::http::{ContentEncoding, ParsedRequest, Response, negotiate_content_encoding};
use crate::routes::IngestRoute;
use crate::server_context::ServerContext;
use crate::util::encode_visitor_timestamp_id;

const TRACKER_JS: &[u8] = include_bytes!("../../../tracker/tracker-min.js");
const TRACKER_JS_GZ: &[u8] = include_bytes!("../../../tracker/tracker-min.js.gz");
const TRACKER_JS_BR: &[u8] = include_bytes!("../../../tracker/tracker-min.js.br");

/// JSON body for the XHR ingest route. Intentionally omits the server pageview
/// id: follow-up hits resolve the page via client `pg` (in-batch map or
/// `client_page_ids_map`), so returning it would only encourage a trust-the-
/// client `spg` shortcut with no real gain.
#[derive(Serialize)]
struct IngestJsonResponse {
    /// Hex-encoded, as the session cookie and `s` param carry it.
    visitor_timestamp_id: Option<String>,
    /// True when cookies and terminal-device reads are lawful for this visitor:
    /// explicit consent, or a jurisdiction that does not require it. The tracker
    /// sets/refreshes the session cookie and may send a `context` hit; the server
    /// does not care whether a session id already exists.
    storage_allowed: bool,
}

/// Entry point for ingest routes. Runs on the connection's async task; the
/// project database is only touched lightly for session/page lookups.
pub(crate) async fn handle_ingest_request(
    route: IngestRoute,
    parsed: &ParsedRequest<'_>,
    body: &[u8],
    client_ip: Option<IpAddr>,
    cf_country: Option<&str>,
    server_context: Arc<ServerContext>,
) -> Response {
    if route == IngestRoute::Js {
        if parsed.method != "GET" && parsed.method != "HEAD" {
            return Response::method_not_allowed();
        }
        return match negotiate_content_encoding(parsed.accept_encoding) {
            ContentEncoding::Brotli => Response::javascript(TRACKER_JS_BR, Some("br")),
            ContentEncoding::Gzip => Response::javascript(TRACKER_JS_GZ, Some("gzip")),
            ContentEncoding::Identity => Response::javascript(TRACKER_JS, None),
        };
    }

    let outcome = match persist_payload(parsed, body, client_ip, cf_country, server_context).await {
        Ok(outcome) => outcome,
        Err(PersistError::InvalidPayload) => return Response::bad_request(),
        Err(PersistError::UnknownProject) => return Response::not_found(),
        Err(
            PersistError::Database(message)
            | PersistError::DatabaseBusy(message)
            | PersistError::Panic(message),
        ) => {
            return Response::internal_server_error()
                .with_internal_error(format!("{INGEST_PERSIST_INTERNAL_ERROR}: {message}"));
        }
    };

    match route {
        IngestRoute::Json => Response::ok_json(IngestJsonResponse {
            visitor_timestamp_id: outcome
                .visitor_timestamp_id
                .map(encode_visitor_timestamp_id),
            storage_allowed: outcome.storage_allowed,
        }),
        IngestRoute::Gif => {
            let project_name = crate::routes::ingest_project_name(parsed.target).unwrap_or("");
            let headers = session_cookie_headers(project_name, outcome.session_cookie.as_ref());
            if headers.is_empty() {
                Response::gif()
            } else {
                Response::gif_with_headers(&headers)
            }
        }
        IngestRoute::Js => unreachable!("js route handled before persist"),
    }
}

async fn persist_payload(
    parsed: &ParsedRequest<'_>,
    body: &[u8],
    client_ip: Option<IpAddr>,
    cf_country: Option<&str>,
    server_context: Arc<ServerContext>,
) -> Result<PersistOutcome, PersistError> {
    let (project_name, items) = match parse_and_validate_payload(parsed.target, body)? {
        ParsedPayloadOutcome::Ignore => return Ok(PersistOutcome::default()),
        ParsedPayloadOutcome::Valid(validated) => {
            (validated.project_name.clone(), vec![*validated])
        }
        ParsedPayloadOutcome::Batch(batch) => (batch.project_name, batch.items),
    };
    let project = crate::projects::resolve_project(server_context.clone(), &project_name)
        .await?
        .ok_or(PersistError::UnknownProject)?;
    let project_id = project.id_string();

    if let Some(client_ip) = client_ip
        && exclude_users::should_exclude_ingest(server_context.clone(), &project, client_ip).await
    {
        return Ok(PersistOutcome::default());
    }

    // Over the limit, a report is dropped rather than refused, so the tracker
    // has nothing to log in the visitor's console.
    if let Some(error) = items.iter().find(|item| item.hit_kind == HitKind::Error) {
        if may_record_error(&server_context, client_ip) {
            persist_error_to_main(server_context, &project_id, error).await?;
        }
        return Ok(PersistOutcome::default());
    }

    let (outcome, constraint_failures) = enqueue_items(
        parsed,
        client_ip,
        cf_country,
        server_context.clone(),
        &project_id,
        &project_name,
        items,
    )?;
    // A client can trip these on purpose, so they share the error report limit.
    for failure in constraint_failures {
        if may_record_error(&server_context, client_ip) {
            persist_row_failure_to_main(server_context.clone(), &project_id, &failure).await?;
        }
    }
    Ok(outcome)
}
