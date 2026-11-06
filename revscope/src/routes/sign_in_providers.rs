//! The client ids the browser signs in to Google and Apple with, so one server
//! setting turns each button on, self-hosted builds included.

use serde_json::json;

use crate::http::Response;

use super::request::{ApiRequest, RouteResult};

pub(super) fn sign_in_providers(request: &ApiRequest) -> RouteResult {
    let context = request.context();
    let client_id = |verifier: Option<&crate::id_token::Verifier>| {
        verifier.map(|verifier| verifier.client_id.clone())
    };
    Ok(Response::ok_json(json!({
        "google": client_id(context.google_sign_in.as_ref()),
        "apple": client_id(context.apple_sign_in.as_ref()),
    })))
}
