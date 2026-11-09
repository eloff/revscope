//! `GET /v2/session/projects/{id}/install` - the snippet to paste into a site.
//!
//! Built here rather than by each client because only the server knows where its
//! tracker is served from: a custom domain, this server when self-hosted or in
//! local development, or otherwise the stub's default of `in.revscope.co`.

use std::sync::Arc;

use revscope_api::install::{InstallResponse, render_snippet};

use crate::auth::AuthError;
use crate::authorize::OwnedProject;
use crate::http::host_supports_secure_cookies;
use crate::mcp::api_base_url;
use crate::projects;
use crate::server_context::ServerContext;
use crate::settings;

pub(crate) fn install(
    server_context: Arc<ServerContext>,
    project: &OwnedProject,
    host: Option<&str>,
) -> Result<InstallResponse, AuthError> {
    let row = projects::fetch_project_blocking(server_context.clone(), project.key())
        .map_err(|error| AuthError::Db(error.to_string()))?
        .ok_or(AuthError::InvalidRequest("project not found"))?;
    let settings = settings::get_settings(server_context, project)?;
    // One kept from before a downgrade stays in the settings so it can be cleared.
    let custom_domain = settings
        .custom_domain
        .filter(|_| settings.custom_domain_included);
    let server_url = server_url(custom_domain.as_deref(), host);
    Ok(InstallResponse {
        snippet: render_snippet(row.name(), server_url.as_deref()),
        project: row.name().to_string(),
        server_url,
    })
}

fn server_url(custom_domain: Option<&str>, host: Option<&str>) -> Option<String> {
    if let Some(domain) = custom_domain {
        return Some(format!("https://{domain}/"));
    }
    (cfg!(feature = "self-hosted") || !host_supports_secure_cookies(host))
        .then(|| format!("{}/", api_base_url(host)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_custom_domain_is_where_the_tracker_reports() {
        assert_eq!(
            server_url(Some("stats.example.com"), Some("api.revscope.co")).as_deref(),
            Some("https://stats.example.com/")
        );
    }

    #[test]
    fn local_development_reports_to_this_server() {
        assert_eq!(
            server_url(None, Some("localhost:4320")).as_deref(),
            Some("http://localhost:4320/")
        );
    }

    #[cfg(not(feature = "self-hosted"))]
    #[test]
    fn the_hosted_service_keeps_the_stubs_default() {
        assert_eq!(server_url(None, Some("api.revscope.co")), None);
    }
}
