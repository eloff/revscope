//! Maps request paths to the route they name. Which credentials each route
//! takes is declared per method in [`super::table`], not here.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IngestRoute {
    Json,
    Gif,
    Js,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ApiRoute {
    Signup,
    Login,
    Logout,
    ForgotPassword,
    ResetPassword,
    VerifyEmail,
    ResendVerification,
    IdTokenSignIn,
    SignInProviders,
    Me,
    #[cfg(feature = "billing")]
    Checkout,
    #[cfg(feature = "billing")]
    PolarWebhook,
    #[cfg(feature = "billing")]
    StripeWebhook,
    Overview,
    Conversions,
    PaymentActivity,
    PaymentJourney,
    RevenueSources,
    SearchTerms,
    Audience,
    Pages,
    PageTransitions,
    LandingPages,
    Visitors,
    VisitorJourney,
    Events,
    Crawlers,
    CrawlerDetail,
    Funnels,
    FunnelsEdit,
    FunnelData,
    FunnelPreview,
    FunnelOptions,
    Campaigns,
    CampaignsEdit,
    CampaignPreview,
    CampaignSpend,
    GaImports,
    GaImportCancel,
    GaImportOauthUrl,
    GaImportConnect,
    GaImportSettings,
    ImportedCoverage,
    Settings,
    #[cfg(feature = "billing")]
    Billing,
    #[cfg(feature = "billing")]
    PlanChange,
    #[cfg(feature = "billing")]
    PlanChangePreview,
    AccountUsers,
    InviteUser,
    UpdateUser,
    RemoveUser,
    CreateProject,
    DeleteProject,
    Install,
    Troubleshooting,
    TroubleshootingCheck,
    TroubleshootingIngest,
    Query,
    QueryToken,
    Schema,
    DownloadDb,
    DownloadDbToken,
    Chat,
    ChatStatus,
    DemoChatToken,
    ChatConversations,
    ChatMessages,
    /// The MCP endpoint itself. Dispatched asynchronously by the server rather
    /// than through [`handle_api_route`](super::handle_api_route), because `rmcp` is async and its answer
    /// may be a stream.
    Mcp,
    McpRegister,
    McpAuthorize,
    McpToken,
    McpRevoke,
}

/// The `/v2/{prefix}/…` segment, which names the family of credentials the
/// route takes. The route table's guard must belong to the same family; the
/// router refuses a route where they disagree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PathPrefix {
    Public,
    Session,
    Read,
    Query,
    Download,
    Chat,
    /// `/v2/mcp` itself, whose access token the MCP transport checks.
    Mcp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedApiRoute {
    pub prefix: PathPrefix,
    pub route: ApiRoute,
    pub project_id: Option<String>,
}

pub(crate) fn resolve_ingest_route(target: &str) -> Option<IngestRoute> {
    let path = target.split('?').next().unwrap_or(target);

    if path == "/v2" || path.starts_with("/v2/") {
        return None;
    }

    let route = if path.ends_with(".js") {
        IngestRoute::Js
    } else if path.ends_with(".gif") {
        IngestRoute::Gif
    } else {
        IngestRoute::Json
    };

    ingest_path_stem(path, route).map(|_| route)
}

/// Project name encoded in a Json/Gif ingest path (`/{name}` or `/{name}.gif`).
pub(crate) fn ingest_project_name(target: &str) -> Option<&str> {
    let path = target.split('?').next().unwrap_or(target);
    let route = if path.ends_with(".gif") {
        IngestRoute::Gif
    } else if path.ends_with(".js") {
        return None;
    } else {
        IngestRoute::Json
    };
    let stem = ingest_path_stem(path, route)?;
    crate::projects::is_valid_project_name(stem).then_some(stem)
}

pub(crate) fn resolve_api_route(target: &str) -> Option<ResolvedApiRoute> {
    let path = target.split('?').next().unwrap_or(target);
    let mut segments = path.split('/').filter(|segment| !segment.is_empty());
    if segments.next() != Some("v2") {
        return None;
    }
    let (prefix, route, project_id) = match segments.next()? {
        // The one route whose prefix is the whole path: an MCP client is
        // configured with a single URL and learns everything else from a 401.
        "mcp" if segments.next().is_none() => (PathPrefix::Mcp, ApiRoute::Mcp, None),
        "public" => (
            PathPrefix::Public,
            resolve_public_route(&mut segments)?,
            None,
        ),
        "session" => {
            let (route, project_id) = resolve_session_route(&mut segments)?;
            (PathPrefix::Session, route, project_id)
        }
        "read" => {
            let project_id = take_project_id(&mut segments)?;
            let mut rest = segments.peekable();
            // Ask's old home. Kept so app bundles built before the move to
            // `/v2/chat/` still work while the API and app deploy separately;
            // remove once no deployed app calls it. Guarded as Ask, not as a read.
            if rest.next_if_eq(&"chat").is_some() {
                let route = resolve_chat_route(&mut rest)?;
                (PathPrefix::Chat, route, Some(project_id))
            } else {
                let route = resolve_read_route(&mut rest)?;
                (PathPrefix::Read, route, Some(project_id))
            }
        }
        "query" => {
            let project_id = take_project_id(&mut segments)?;
            let route = resolve_query_route(&mut segments)?;
            (PathPrefix::Query, route, Some(project_id))
        }
        "download" => {
            let project_id = take_project_id(&mut segments)?;
            if segments.next().is_some() {
                return None;
            }
            (PathPrefix::Download, ApiRoute::DownloadDb, Some(project_id))
        }
        "chat" => {
            let project_id = take_project_id(&mut segments)?;
            let route = resolve_chat_route(&mut segments)?;
            (PathPrefix::Chat, route, Some(project_id))
        }
        _ => return None,
    };
    Some(ResolvedApiRoute {
        prefix,
        route,
        project_id,
    })
}

fn take_project_id<'a>(segments: &mut impl Iterator<Item = &'a str>) -> Option<String> {
    if segments.next() != Some("projects") {
        return None;
    }
    let project_id = segments.next()?;
    if !crate::projects::is_valid_project_id(project_id) {
        return None;
    }
    Some(crate::projects::normalize_project_id(project_id))
}

fn resolve_public_route<'a>(segments: &mut impl Iterator<Item = &'a str>) -> Option<ApiRoute> {
    match (segments.next(), segments.next(), segments.next()) {
        (Some("auth"), Some("signup"), None) => Some(ApiRoute::Signup),
        (Some("auth"), Some("login"), None) => Some(ApiRoute::Login),
        (Some("auth"), Some("logout"), None) => Some(ApiRoute::Logout),
        (Some("auth"), Some("forgot-password"), None) => Some(ApiRoute::ForgotPassword),
        (Some("auth"), Some("reset-password"), None) => Some(ApiRoute::ResetPassword),
        (Some("auth"), Some("verify-email"), None) => Some(ApiRoute::VerifyEmail),
        (Some("auth"), Some("id-token"), None) => Some(ApiRoute::IdTokenSignIn),
        (Some("auth"), Some("providers"), None) => Some(ApiRoute::SignInProviders),
        #[cfg(feature = "billing")]
        (Some("webhooks"), Some("polar"), None) => Some(ApiRoute::PolarWebhook),
        #[cfg(feature = "billing")]
        (Some("webhooks"), Some("stripe"), None) => Some(ApiRoute::StripeWebhook),
        (Some("chat"), Some("status"), None) => Some(ApiRoute::ChatStatus),
        (Some("chat"), Some("demo-token"), None) => Some(ApiRoute::DemoChatToken),
        // Both OAuth endpoints an MCP client calls without a user present: it
        // registers itself before the consent page, and redeems its code after.
        (Some("mcp"), Some("register"), None) => Some(ApiRoute::McpRegister),
        (Some("mcp"), Some("token"), None) => Some(ApiRoute::McpToken),
        _ => None,
    }
}

fn resolve_session_route<'a>(
    segments: &mut impl Iterator<Item = &'a str>,
) -> Option<(ApiRoute, Option<String>)> {
    let first = segments.next()?;
    let route = match first {
        "me" => match (segments.next(), segments.next()) {
            (None, _) => ApiRoute::Me,
            (Some("verify-email"), None) => ApiRoute::ResendVerification,
            _ => return None,
        },
        #[cfg(feature = "billing")]
        "checkout" if segments.next().is_none() => ApiRoute::Checkout,
        #[cfg(feature = "billing")]
        "billing" => match (segments.next(), segments.next(), segments.next()) {
            (None, _, _) => ApiRoute::Billing,
            (Some("plan"), None, _) => ApiRoute::PlanChange,
            (Some("plan"), Some("preview"), None) => ApiRoute::PlanChangePreview,
            _ => return None,
        },
        "users" => match (segments.next(), segments.next()) {
            (None, _) => ApiRoute::AccountUsers,
            (Some("invite"), None) => ApiRoute::InviteUser,
            (Some("update"), None) => ApiRoute::UpdateUser,
            (Some("remove"), None) => ApiRoute::RemoveUser,
            _ => return None,
        },
        // Session-authed because only a signed-in browser may approve an MCP
        // client or revoke the tokens that approval minted.
        "mcp" => match (segments.next(), segments.next()) {
            (Some("authorize"), None) => ApiRoute::McpAuthorize,
            (Some("revoke"), None) => ApiRoute::McpRevoke,
            _ => return None,
        },
        "projects" => {
            let Some(id) = segments.next() else {
                return Some((ApiRoute::CreateProject, None));
            };
            if !crate::projects::is_valid_project_id(id) {
                return None;
            }
            let project_id = crate::projects::normalize_project_id(id);
            let rest: Vec<&str> = segments.collect();
            let route = match rest.as_slice() {
                [] => ApiRoute::DeleteProject,
                ["settings"] => ApiRoute::Settings,
                ["install"] => ApiRoute::Install,
                ["troubleshooting"] => ApiRoute::Troubleshooting,
                ["troubleshooting", "check"] => ApiRoute::TroubleshootingCheck,
                ["troubleshooting", "ingest"] => ApiRoute::TroubleshootingIngest,
                ["query-token"] => ApiRoute::QueryToken,
                ["download-token"] => ApiRoute::DownloadDbToken,
                ["funnels"] => ApiRoute::FunnelsEdit,
                ["campaigns"] => ApiRoute::CampaignsEdit,
                ["campaign-spend"] => ApiRoute::CampaignSpend,
                ["ga-imports"] => ApiRoute::GaImports,
                ["ga-imports", "cancel"] => ApiRoute::GaImportCancel,
                ["ga-imports", "oauth-url"] => ApiRoute::GaImportOauthUrl,
                ["ga-imports", "connect"] => ApiRoute::GaImportConnect,
                ["ga-imports", "settings"] => ApiRoute::GaImportSettings,
                _ => return None,
            };
            return Some((route, Some(project_id)));
        }
        _ => return None,
    };
    Some((route, None))
}

fn resolve_read_route<'a>(segments: &mut impl Iterator<Item = &'a str>) -> Option<ApiRoute> {
    let rest: Vec<&str> = segments.collect();
    Some(match rest.as_slice() {
        ["overview"] => ApiRoute::Overview,
        ["imported-coverage"] => ApiRoute::ImportedCoverage,
        ["conversions"] => ApiRoute::Conversions,
        ["payment-activity"] => ApiRoute::PaymentActivity,
        ["payment-journey"] => ApiRoute::PaymentJourney,
        ["revenue-sources"] => ApiRoute::RevenueSources,
        ["search-terms"] => ApiRoute::SearchTerms,
        ["audience"] => ApiRoute::Audience,
        ["pages"] => ApiRoute::Pages,
        ["page-transitions"] => ApiRoute::PageTransitions,
        ["landing-pages"] => ApiRoute::LandingPages,
        ["visitors"] => ApiRoute::Visitors,
        ["visitor-journey"] => ApiRoute::VisitorJourney,
        ["events"] => ApiRoute::Events,
        ["crawlers"] => ApiRoute::Crawlers,
        ["crawler-detail"] => ApiRoute::CrawlerDetail,
        ["funnels"] => ApiRoute::Funnels,
        ["funnel-data"] => ApiRoute::FunnelData,
        ["funnel-preview"] => ApiRoute::FunnelPreview,
        ["funnel-options"] => ApiRoute::FunnelOptions,
        ["campaigns"] => ApiRoute::Campaigns,
        ["campaign-preview"] => ApiRoute::CampaignPreview,
        _ => return None,
    })
}

fn resolve_query_route<'a>(segments: &mut impl Iterator<Item = &'a str>) -> Option<ApiRoute> {
    let rest: Vec<&str> = segments.collect();
    Some(match rest.as_slice() {
        ["sql"] => ApiRoute::Query,
        ["schema"] => ApiRoute::Schema,
        _ => return None,
    })
}

fn resolve_chat_route<'a>(segments: &mut impl Iterator<Item = &'a str>) -> Option<ApiRoute> {
    let rest: Vec<&str> = segments.collect();
    Some(match rest.as_slice() {
        [] => ApiRoute::Chat,
        ["conversations"] => ApiRoute::ChatConversations,
        ["messages"] => ApiRoute::ChatMessages,
        _ => return None,
    })
}

fn ingest_path_stem(path: &str, route: IngestRoute) -> Option<&str> {
    let decoration = path.strip_prefix('/')?;

    match route {
        IngestRoute::Js => {
            let stem = decoration.strip_suffix(".js")?;
            crate::projects::is_valid_project_name(stem).then_some(stem)
        }
        IngestRoute::Gif => {
            let stem = decoration.strip_suffix(".gif")?;
            crate::projects::is_valid_project_name(stem).then_some(stem)
        }
        IngestRoute::Json => {
            crate::projects::is_valid_project_name(decoration).then_some(decoration)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolved(
        route: ApiRoute,
        prefix: PathPrefix,
        project_id: Option<String>,
    ) -> ResolvedApiRoute {
        ResolvedApiRoute {
            prefix,
            route,
            project_id,
        }
    }

    #[test]
    fn resolves_ingest_routes_without_v2_prefix() {
        assert_eq!(resolve_ingest_route("/demo"), Some(IngestRoute::Json));
        assert_eq!(resolve_ingest_route("/my_project"), Some(IngestRoute::Json));
        assert_eq!(resolve_ingest_route("/demo.gif"), Some(IngestRoute::Gif));
        assert_eq!(
            resolve_ingest_route("/my_project.gif"),
            Some(IngestRoute::Gif)
        );
        assert_eq!(resolve_ingest_route("/demo.js"), Some(IngestRoute::Js));
        assert_eq!(
            resolve_ingest_route("/my_project.js"),
            Some(IngestRoute::Js)
        );
    }

    #[test]
    fn extracts_project_name_from_ingest_path() {
        assert_eq!(ingest_project_name("/demo"), Some("demo"));
        assert_eq!(ingest_project_name("/demo?ty=pageview"), Some("demo"));
        assert_eq!(ingest_project_name("/demo.gif"), Some("demo"));
        assert_eq!(
            ingest_project_name("/my_project.gif?x=1"),
            Some("my_project")
        );
        assert_eq!(ingest_project_name("/script.js"), None);
        assert_eq!(ingest_project_name("/"), None);
        assert_eq!(ingest_project_name("/ab"), None);
    }

    #[test]
    fn rejects_v2_paths_as_ingest() {
        assert_eq!(resolve_ingest_route("/v2"), None);
        assert_eq!(resolve_ingest_route("/v2/public/auth/login"), None);
    }

    #[test]
    fn rejects_invalid_ingest_paths() {
        assert_eq!(resolve_ingest_route("/"), None);
        assert_eq!(resolve_ingest_route("/ab"), None);
        assert_eq!(resolve_ingest_route("/ab/cd"), None);
        assert_eq!(resolve_ingest_route("/NOPE"), None);
        assert_eq!(resolve_ingest_route("/bad-chars"), None);
        assert_eq!(resolve_ingest_route("/ga.gif"), None);
        assert_eq!(resolve_ingest_route("/ab.js"), None);
        assert_eq!(resolve_ingest_route("/ab/cd.js"), None);
        assert_eq!(resolve_ingest_route("/bad-chars.js"), None);
    }

    #[test]
    fn resolves_auth_api_routes() {
        assert_eq!(
            resolve_api_route("/v2/public/auth/signup"),
            Some(resolved(ApiRoute::Signup, PathPrefix::Public, None))
        );
        assert_eq!(
            resolve_api_route("/v2/public/auth/login?next=%2Fdashboard"),
            Some(resolved(ApiRoute::Login, PathPrefix::Public, None))
        );
        assert_eq!(
            resolve_api_route("/v2/public/auth/forgot-password"),
            Some(resolved(ApiRoute::ForgotPassword, PathPrefix::Public, None))
        );
        assert_eq!(
            resolve_api_route("/v2/public/auth/reset-password"),
            Some(resolved(ApiRoute::ResetPassword, PathPrefix::Public, None))
        );
        assert_eq!(
            resolve_api_route("/v2/public/auth/logout"),
            Some(resolved(ApiRoute::Logout, PathPrefix::Public, None))
        );
        assert_eq!(
            resolve_api_route("/v2/public/auth/id-token"),
            Some(resolved(ApiRoute::IdTokenSignIn, PathPrefix::Public, None))
        );
        assert_eq!(
            resolve_api_route("/v2/public/auth/providers"),
            Some(resolved(
                ApiRoute::SignInProviders,
                PathPrefix::Public,
                None
            ))
        );
        #[cfg(feature = "billing")]
        assert_eq!(
            resolve_api_route("/v2/public/webhooks/polar"),
            Some(resolved(ApiRoute::PolarWebhook, PathPrefix::Public, None))
        );
        #[cfg(feature = "billing")]
        assert_eq!(
            resolve_api_route("/v2/public/webhooks/stripe"),
            Some(resolved(ApiRoute::StripeWebhook, PathPrefix::Public, None))
        );
        assert_eq!(
            resolve_api_route("/v2/public/chat/status"),
            Some(resolved(ApiRoute::ChatStatus, PathPrefix::Public, None))
        );
        assert_eq!(
            resolve_api_route("/v2/public/chat/demo-token"),
            Some(resolved(ApiRoute::DemoChatToken, PathPrefix::Public, None))
        );
        assert_eq!(
            resolve_api_route("/v2/session/me"),
            Some(resolved(ApiRoute::Me, PathPrefix::Session, None))
        );
        assert_eq!(
            resolve_api_route("/v2/session/projects"),
            Some(resolved(ApiRoute::CreateProject, PathPrefix::Session, None))
        );
        assert_eq!(
            resolve_api_route("/v2/session/projects/07"),
            Some(resolved(
                ApiRoute::DeleteProject,
                PathPrefix::Session,
                Some("7".to_string())
            ))
        );
        assert_eq!(resolve_api_route("/v2/session/projects/abc"), None);
        #[cfg(feature = "billing")]
        assert_eq!(
            resolve_api_route("/v2/session/checkout"),
            Some(resolved(ApiRoute::Checkout, PathPrefix::Session, None))
        );
        #[cfg(feature = "billing")]
        assert_eq!(
            resolve_api_route("/v2/session/billing"),
            Some(resolved(ApiRoute::Billing, PathPrefix::Session, None))
        );
        #[cfg(feature = "billing")]
        assert_eq!(
            resolve_api_route("/v2/session/billing/plan"),
            Some(resolved(ApiRoute::PlanChange, PathPrefix::Session, None))
        );
        #[cfg(feature = "billing")]
        assert_eq!(
            resolve_api_route("/v2/session/billing/plan/preview"),
            Some(resolved(
                ApiRoute::PlanChangePreview,
                PathPrefix::Session,
                None
            ))
        );
    }

    #[test]
    fn resolves_the_mcp_endpoint_and_its_oauth_routes() {
        // The endpoint an MCP client is configured with. No project in the path:
        // the token names it.
        assert_eq!(
            resolve_api_route("/v2/mcp"),
            Some(resolved(ApiRoute::Mcp, PathPrefix::Mcp, None))
        );
        assert_eq!(
            resolve_api_route("/v2/public/mcp/register"),
            Some(resolved(ApiRoute::McpRegister, PathPrefix::Public, None))
        );
        assert_eq!(
            resolve_api_route("/v2/public/mcp/token"),
            Some(resolved(ApiRoute::McpToken, PathPrefix::Public, None))
        );
        assert_eq!(
            resolve_api_route("/v2/session/mcp/authorize?client_id=abc"),
            Some(resolved(ApiRoute::McpAuthorize, PathPrefix::Session, None))
        );
        assert_eq!(
            resolve_api_route("/v2/session/mcp/revoke"),
            Some(resolved(ApiRoute::McpRevoke, PathPrefix::Session, None))
        );

        // Nothing hangs off the endpoint, and consent is not a public route.
        assert_eq!(resolve_api_route("/v2/mcp/tools"), None);
        assert_eq!(resolve_api_route("/v2/public/mcp/authorize"), None);
        assert_eq!(resolve_api_route("/v2/session/mcp"), None);
    }

    #[test]
    fn resolves_project_routes_with_prefix_and_id() {
        assert_eq!(
            resolve_api_route("/v2/read/projects/1/overview?start=1&end=2"),
            Some(resolved(
                ApiRoute::Overview,
                PathPrefix::Read,
                Some("1".to_string())
            ))
        );
        assert_eq!(
            resolve_api_route("/v2/read/projects/1/crawlers"),
            Some(resolved(
                ApiRoute::Crawlers,
                PathPrefix::Read,
                Some("1".to_string())
            ))
        );
        assert_eq!(
            resolve_api_route("/v2/read/projects/1/crawler-detail"),
            Some(resolved(
                ApiRoute::CrawlerDetail,
                PathPrefix::Read,
                Some("1".to_string())
            ))
        );
        assert_eq!(
            resolve_api_route("/v2/read/projects/42/payment-activity"),
            Some(resolved(
                ApiRoute::PaymentActivity,
                PathPrefix::Read,
                Some("42".to_string())
            ))
        );
        assert_eq!(
            resolve_api_route("/v2/read/projects/1/funnels"),
            Some(resolved(
                ApiRoute::Funnels,
                PathPrefix::Read,
                Some("1".to_string())
            ))
        );
        assert_eq!(
            resolve_api_route("/v2/read/projects/1/campaigns?start=1&end=2"),
            Some(resolved(
                ApiRoute::Campaigns,
                PathPrefix::Read,
                Some("1".to_string())
            ))
        );
        assert_eq!(
            resolve_api_route("/v2/read/projects/1/campaign-preview"),
            Some(resolved(
                ApiRoute::CampaignPreview,
                PathPrefix::Read,
                Some("1".to_string())
            ))
        );
        assert_eq!(
            resolve_api_route("/v2/session/projects/1/campaigns"),
            Some(resolved(
                ApiRoute::CampaignsEdit,
                PathPrefix::Session,
                Some("1".to_string())
            ))
        );
        assert_eq!(
            resolve_api_route("/v2/session/projects/1/campaign-spend"),
            Some(resolved(
                ApiRoute::CampaignSpend,
                PathPrefix::Session,
                Some("1".to_string())
            ))
        );
        assert_eq!(
            resolve_api_route("/v2/session/projects/1/funnels"),
            Some(resolved(
                ApiRoute::FunnelsEdit,
                PathPrefix::Session,
                Some("1".to_string())
            ))
        );
        assert_eq!(
            resolve_api_route("/v2/session/projects/1/settings"),
            Some(resolved(
                ApiRoute::Settings,
                PathPrefix::Session,
                Some("1".to_string())
            ))
        );
        assert_eq!(
            resolve_api_route("/v2/session/projects/1/query-token"),
            Some(resolved(
                ApiRoute::QueryToken,
                PathPrefix::Session,
                Some("1".to_string())
            ))
        );
        assert_eq!(
            resolve_api_route("/v2/session/projects/1/download-token"),
            Some(resolved(
                ApiRoute::DownloadDbToken,
                PathPrefix::Session,
                Some("1".to_string())
            ))
        );
        assert_eq!(
            resolve_api_route("/v2/query/projects/1/sql"),
            Some(resolved(
                ApiRoute::Query,
                PathPrefix::Query,
                Some("1".to_string())
            ))
        );
        assert_eq!(
            resolve_api_route("/v2/query/projects/1/schema"),
            Some(resolved(
                ApiRoute::Schema,
                PathPrefix::Query,
                Some("1".to_string())
            ))
        );
        assert_eq!(
            resolve_api_route("/v2/download/projects/1?token=abc"),
            Some(resolved(
                ApiRoute::DownloadDb,
                PathPrefix::Download,
                Some("1".to_string())
            ))
        );
        assert_eq!(
            resolve_api_route("/v2/chat/projects/1"),
            Some(resolved(
                ApiRoute::Chat,
                PathPrefix::Chat,
                Some("1".to_string())
            ))
        );
        assert_eq!(
            resolve_api_route("/v2/chat/projects/1/conversations"),
            Some(resolved(
                ApiRoute::ChatConversations,
                PathPrefix::Chat,
                Some("1".to_string())
            ))
        );
        assert_eq!(
            resolve_api_route("/v2/chat/projects/1/messages"),
            Some(resolved(
                ApiRoute::ChatMessages,
                PathPrefix::Chat,
                Some("1".to_string())
            ))
        );
        assert_eq!(
            resolve_api_route("/v2/read/projects/01/overview"),
            Some(resolved(
                ApiRoute::Overview,
                PathPrefix::Read,
                Some("1".to_string())
            ))
        );
        assert_eq!(
            resolve_api_route("/v2/session/projects/1/troubleshooting/check"),
            Some(resolved(
                ApiRoute::TroubleshootingCheck,
                PathPrefix::Session,
                Some("1".to_string())
            ))
        );
        assert_eq!(
            resolve_api_route("/v2/session/projects/1/troubleshooting/ingest"),
            Some(resolved(
                ApiRoute::TroubleshootingIngest,
                PathPrefix::Session,
                Some("1".to_string())
            ))
        );
    }

    #[test]
    #[cfg(not(feature = "billing"))]
    fn builds_without_billing_have_no_billing_routes() {
        for path in [
            "/v2/public/webhooks/polar",
            "/v2/public/webhooks/stripe",
            "/v2/session/checkout",
            "/v2/session/billing",
        ] {
            assert_eq!(resolve_api_route(path), None, "{path}");
        }
    }

    #[test]
    fn old_ask_paths_resolve_to_ask_and_its_policy() {
        for (path, route) in [
            ("/v2/read/projects/1/chat", ApiRoute::Chat),
            (
                "/v2/read/projects/1/chat/conversations",
                ApiRoute::ChatConversations,
            ),
            ("/v2/read/projects/1/chat/messages", ApiRoute::ChatMessages),
        ] {
            assert_eq!(
                resolve_api_route(path),
                Some(resolved(route, PathPrefix::Chat, Some("1".to_string()))),
                "{path}"
            );
        }
    }

    #[test]
    fn rejects_old_api_paths() {
        assert_eq!(resolve_api_route("/v2/api/me"), None);
        assert_eq!(resolve_api_route("/v2/api/overview"), None);
        assert_eq!(resolve_api_route("/v2/api/query"), None);
        assert_eq!(resolve_api_route("/v2/auth/login"), None);
        assert_eq!(resolve_api_route("/v2/webhooks/polar"), None);
        assert_eq!(resolve_api_route("/v2/read/projects/abc/overview"), None);
        assert_eq!(resolve_api_route("/v2/read/projects/1/unknown"), None);
        assert_eq!(resolve_api_route("/v2/read/projects/1/chat/unknown"), None);
    }
}
