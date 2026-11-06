//! Which credentials every route takes, per method, and the handler they admit.
//!
//! Each [`AuthPolicy`] variant carries a handler whose signature demands the
//! grant that policy produces, so a route cannot be registered without
//! declaring its policy. Policies under one path prefix all hand over
//! different types, and the router refuses a policy from another prefix, so
//! a handler cannot be paired with a policy weaker than the one it was written
//! for. [`methods`] matches every [`ApiRoute`] without a wildcard, so a new
//! route doesn't compile until it has an entry.

use crate::authorize::{
    Account, AdminAccount, AdminProject, AuthorizedProject, ChatGrant, DownloadableProject,
    OwnedProject, WritableProject,
};

use super::query::QueryCaller;
use super::request::{ApiRequest, RouteResult};
use super::{
    ApiRoute, PathPrefix, account, campaigns, chat, dashboard, download, funnels, ga_import,
    id_token_sign_in, mcp, project_admin, query, sign_in_providers,
};
#[cfg(feature = "billing")]
use crate::billing::{routes as billing, webhook_routes as webhooks};

pub(super) enum AuthPolicy {
    /// No credentials. Whatever the handler needs to trust (a webhook
    /// signature, a password, an OAuth code) it checks itself.
    Public(fn(&ApiRequest) -> RouteResult),
    /// A live session, for routes that act on the caller's own account.
    Account(fn(&ApiRequest, &Account) -> RouteResult),
    /// A live session of an admin of their account: billing, users, adding
    /// projects.
    Admin(fn(&ApiRequest, &AdminAccount) -> RouteResult),
    /// An admin changing their account's project, which the demo refuses.
    AdminProjectWrite(fn(&ApiRequest, &AdminProject) -> RouteResult),
    /// A live session on the account that owns the path's project.
    OwnProject(fn(&ApiRequest, &Account, &OwnedProject) -> RouteResult),
    /// Like [`Self::OwnProject`], but also the public demo.
    OwnProjectOrDemo(fn(&ApiRequest, &Account, &DownloadableProject) -> RouteResult),
    /// Like [`Self::OwnProject`], to change the project, which the demo refuses.
    OwnProjectWrite(fn(&ApiRequest, &Account, &WritableProject) -> RouteResult),
    /// The owner's session; for the public demo, any session or none.
    Read(fn(&ApiRequest, &AuthorizedProject) -> RouteResult),
    /// The owner's session or a query token minted for the project.
    Query(fn(&ApiRequest, &QueryCaller) -> RouteResult),
    /// The owner's session, any session for the demo, or a download token.
    Download(fn(&ApiRequest, &DownloadableProject) -> RouteResult),
    /// The owner's session, any session for the demo, or an anonymous visitor
    /// with a client IP for the demo.
    Chat(fn(&ApiRequest, &ChatGrant) -> RouteResult),
}

impl AuthPolicy {
    /// The path prefix routes guarded this way live under.
    pub(super) fn prefix(&self) -> PathPrefix {
        match self {
            Self::Public(_) => PathPrefix::Public,
            Self::Account(_)
            | Self::Admin(_)
            | Self::AdminProjectWrite(_)
            | Self::OwnProject(_)
            | Self::OwnProjectOrDemo(_)
            | Self::OwnProjectWrite(_) => PathPrefix::Session,
            Self::Read(_) => PathPrefix::Read,
            Self::Query(_) => PathPrefix::Query,
            Self::Download(_) => PathPrefix::Download,
            Self::Chat(_) => PathPrefix::Chat,
        }
    }
}

type Methods = &'static [(&'static str, AuthPolicy)];

/// The methods `route` answers, each with its policy and handler. Empty only
/// for `/v2/mcp`, which the server answers before this table.
pub(super) fn methods(route: ApiRoute) -> Methods {
    use AuthPolicy::*;
    match route {
        ApiRoute::Signup => &[
            ("GET", Public(account::signup_status)),
            ("POST", Public(account::signup)),
        ],
        ApiRoute::Login => &[("POST", Public(account::login))],
        ApiRoute::Logout => &[("POST", Public(account::logout))],
        ApiRoute::ForgotPassword => &[("POST", Public(account::forgot_password))],
        ApiRoute::ResetPassword => &[("POST", Public(account::reset_password))],
        ApiRoute::IdTokenSignIn => &[("POST", Public(id_token_sign_in::id_token_sign_in))],
        ApiRoute::SignInProviders => &[("GET", Public(sign_in_providers::sign_in_providers))],
        ApiRoute::Me => &[("GET", Account(account::me))],
        #[cfg(feature = "billing")]
        ApiRoute::Checkout => &[("POST", Admin(billing::checkout))],
        #[cfg(feature = "billing")]
        ApiRoute::Billing => &[("GET", Admin(billing::billing))],
        #[cfg(feature = "billing")]
        ApiRoute::PlanChange => &[("POST", Admin(billing::plan_change))],
        #[cfg(feature = "billing")]
        ApiRoute::PlanChangePreview => &[("POST", Admin(billing::plan_change_preview))],
        #[cfg(feature = "billing")]
        ApiRoute::PolarWebhook => &[("POST", Public(webhooks::polar))],
        #[cfg(feature = "billing")]
        ApiRoute::StripeWebhook => &[("POST", Public(webhooks::stripe))],
        ApiRoute::Overview => &[("GET", Read(dashboard::overview))],
        ApiRoute::Conversions => &[("GET", Read(dashboard::conversions))],
        ApiRoute::PaymentActivity => &[("GET", Read(dashboard::payment_activity))],
        ApiRoute::PaymentJourney => &[("GET", Read(dashboard::payment_journey))],
        ApiRoute::RevenueSources => &[("GET", Read(dashboard::revenue_sources))],
        ApiRoute::SearchTerms => &[("GET", Read(dashboard::search_terms))],
        ApiRoute::Audience => &[("GET", Read(dashboard::audience))],
        ApiRoute::Pages => &[("GET", Read(dashboard::pages))],
        ApiRoute::PageTransitions => &[("GET", Read(dashboard::page_transitions))],
        ApiRoute::LandingPages => &[("GET", Read(dashboard::landing_pages))],
        ApiRoute::Visitors => &[("GET", Read(dashboard::visitors))],
        ApiRoute::VisitorJourney => &[("GET", Read(dashboard::visitor_journey))],
        ApiRoute::Events => &[("GET", Read(dashboard::events))],
        ApiRoute::Crawlers => &[("GET", Read(dashboard::crawlers))],
        ApiRoute::CrawlerDetail => &[("GET", Read(dashboard::crawler_detail))],
        ApiRoute::Funnels => &[("GET", Read(funnels::funnels))],
        ApiRoute::FunnelsEdit => &[
            ("POST", OwnProjectWrite(funnels::save_funnel)),
            ("DELETE", OwnProjectWrite(funnels::delete_funnel)),
        ],
        ApiRoute::FunnelData => &[("GET", Read(funnels::funnel_data))],
        ApiRoute::FunnelPreview => &[("POST", Read(funnels::funnel_preview))],
        ApiRoute::FunnelOptions => &[("GET", Read(funnels::funnel_options))],
        ApiRoute::Campaigns => &[("GET", Read(campaigns::campaigns))],
        ApiRoute::CampaignsEdit => &[
            ("POST", OwnProjectWrite(campaigns::save_campaign)),
            ("DELETE", OwnProjectWrite(campaigns::delete_campaign)),
        ],
        ApiRoute::CampaignPreview => &[("POST", Read(campaigns::campaign_preview))],
        ApiRoute::CampaignSpend => &[("POST", OwnProjectWrite(campaigns::campaign_spend))],
        ApiRoute::GaImports => &[
            ("GET", AdminProjectWrite(ga_import::list)),
            ("POST", AdminProjectWrite(ga_import::start)),
            ("DELETE", AdminProjectWrite(ga_import::delete)),
        ],
        ApiRoute::GaImportCancel => &[("POST", AdminProjectWrite(ga_import::cancel))],
        ApiRoute::GaImportOauthUrl => &[("POST", AdminProjectWrite(ga_import::oauth_url))],
        ApiRoute::GaImportConnect => &[("POST", AdminProjectWrite(ga_import::connect))],
        ApiRoute::GaImportSettings => &[("POST", AdminProjectWrite(ga_import::save_settings))],
        ApiRoute::ImportedCoverage => &[("GET", Read(ga_import::coverage))],
        ApiRoute::Settings => &[
            ("GET", OwnProject(project_admin::get_settings)),
            ("POST", AdminProjectWrite(project_admin::save_settings)),
        ],
        ApiRoute::AccountUsers => &[("GET", Admin(project_admin::account_users))],
        ApiRoute::InviteUser => &[("POST", Admin(project_admin::invite_user))],
        ApiRoute::UpdateUser => &[("POST", Admin(project_admin::update_user))],
        ApiRoute::RemoveUser => &[("POST", Admin(project_admin::remove_user))],
        ApiRoute::CreateProject => &[("POST", Admin(project_admin::create_project))],
        ApiRoute::DeleteProject => &[("DELETE", AdminProjectWrite(project_admin::delete_project))],
        ApiRoute::Install => &[("GET", OwnProject(project_admin::install))],
        ApiRoute::Troubleshooting => &[("GET", OwnProject(project_admin::troubleshooting))],
        ApiRoute::TroubleshootingCheck => {
            &[("POST", OwnProject(project_admin::troubleshooting_check))]
        }
        ApiRoute::TroubleshootingIngest => {
            &[("GET", OwnProject(project_admin::troubleshooting_ingest))]
        }
        ApiRoute::Query => &[("POST", Query(query::query))],
        ApiRoute::QueryToken => &[("POST", OwnProject(query::query_token))],
        ApiRoute::Schema => &[("GET", Query(query::schema))],
        ApiRoute::DownloadDb => &[("GET", Download(download::download_db))],
        ApiRoute::DownloadDbToken => &[("POST", OwnProjectOrDemo(download::download_db_token))],
        ApiRoute::ChatStatus => &[("GET", Public(chat::chat_status))],
        ApiRoute::DemoChatToken => &[("POST", Public(chat::demo_chat_token))],
        ApiRoute::Chat => &[("POST", Chat(chat::chat))],
        ApiRoute::ChatConversations => &[
            ("GET", Chat(chat::chat_conversations)),
            ("DELETE", Chat(chat::delete_chat_conversation)),
        ],
        ApiRoute::ChatMessages => &[("GET", Chat(chat::chat_messages))],
        ApiRoute::McpRegister => &[("POST", Public(mcp::register))],
        ApiRoute::McpToken => &[("POST", Public(mcp::token))],
        ApiRoute::McpAuthorize => &[
            ("GET", Account(mcp::describe_client)),
            ("POST", Account(mcp::authorize)),
        ],
        ApiRoute::McpRevoke => &[("POST", Admin(mcp::revoke))],
        ApiRoute::Mcp => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::resolve_api_route;

    /// One path per route the resolver knows, in both builds.
    const PATHS: &[&str] = &[
        "/v2/public/auth/signup",
        "/v2/public/auth/login",
        "/v2/public/auth/logout",
        "/v2/public/auth/forgot-password",
        "/v2/public/auth/reset-password",
        "/v2/public/chat/status",
        "/v2/public/chat/demo-token",
        "/v2/public/mcp/register",
        "/v2/public/mcp/token",
        "/v2/public/webhooks/polar",
        "/v2/public/webhooks/stripe",
        "/v2/session/me",
        "/v2/session/checkout",
        "/v2/session/billing",
        "/v2/session/billing/plan",
        "/v2/session/billing/plan/preview",
        "/v2/session/users",
        "/v2/session/users/invite",
        "/v2/session/users/update",
        "/v2/session/users/remove",
        "/v2/session/mcp/authorize",
        "/v2/session/mcp/revoke",
        "/v2/session/projects",
        "/v2/session/projects/1",
        "/v2/session/projects/1/settings",
        "/v2/session/projects/1/install",
        "/v2/session/projects/1/troubleshooting",
        "/v2/session/projects/1/troubleshooting/check",
        "/v2/session/projects/1/troubleshooting/ingest",
        "/v2/session/projects/1/query-token",
        "/v2/session/projects/1/download-token",
        "/v2/session/projects/1/funnels",
        "/v2/session/projects/1/campaigns",
        "/v2/session/projects/1/campaign-spend",
        "/v2/session/projects/1/ga-imports",
        "/v2/session/projects/1/ga-imports/cancel",
        "/v2/session/projects/1/ga-imports/oauth-url",
        "/v2/session/projects/1/ga-imports/connect",
        "/v2/session/projects/1/ga-imports/settings",
        "/v2/read/projects/1/imported-coverage",
        "/v2/read/projects/1/overview",
        "/v2/read/projects/1/conversions",
        "/v2/read/projects/1/payment-activity",
        "/v2/read/projects/1/payment-journey",
        "/v2/read/projects/1/revenue-sources",
        "/v2/read/projects/1/search-terms",
        "/v2/read/projects/1/audience",
        "/v2/read/projects/1/pages",
        "/v2/read/projects/1/page-transitions",
        "/v2/read/projects/1/landing-pages",
        "/v2/read/projects/1/visitors",
        "/v2/read/projects/1/visitor-journey",
        "/v2/read/projects/1/events",
        "/v2/read/projects/1/crawlers",
        "/v2/read/projects/1/crawler-detail",
        "/v2/read/projects/1/funnels",
        "/v2/read/projects/1/funnel-data",
        "/v2/read/projects/1/funnel-preview",
        "/v2/read/projects/1/funnel-options",
        "/v2/read/projects/1/campaigns",
        "/v2/read/projects/1/campaign-preview",
        "/v2/query/projects/1/sql",
        "/v2/query/projects/1/schema",
        "/v2/download/projects/1",
        "/v2/chat/projects/1",
        "/v2/chat/projects/1/conversations",
        "/v2/chat/projects/1/messages",
        "/v2/read/projects/1/chat",
        "/v2/read/projects/1/chat/conversations",
        "/v2/read/projects/1/chat/messages",
    ];

    /// Paths that resolve only in a build with the private billing module.
    fn resolves_in_this_edition(path: &str) -> bool {
        let billing_only = path.starts_with("/v2/public/webhooks/")
            || path.starts_with("/v2/session/checkout")
            || path.starts_with("/v2/session/billing");
        !billing_only || cfg!(feature = "billing")
    }

    #[test]
    fn every_policy_lives_under_its_own_prefix() {
        for path in PATHS {
            let resolved = resolve_api_route(path);
            assert_eq!(
                resolved.is_some(),
                resolves_in_this_edition(path),
                "{path} resolved to {resolved:?}"
            );
            let Some(resolved) = resolved else {
                continue;
            };
            let methods = methods(resolved.route);
            assert!(!methods.is_empty(), "{path} has no methods");
            for (method, policy) in methods {
                assert_eq!(
                    policy.prefix(),
                    resolved.prefix,
                    "{method} {path} is guarded by a policy from another prefix"
                );
            }
        }
    }

    #[test]
    fn every_route_in_the_shared_contract_exists_with_its_method() {
        for (method, path) in revscope_api::paths::all_routes() {
            let resolved = resolve_api_route(&path)
                .unwrap_or_else(|| panic!("{path} is in the contract but has no route"));
            assert!(
                methods(resolved.route)
                    .iter()
                    .any(|(allowed, _)| *allowed == method),
                "{method} {path} is in the contract but the route doesn't answer it"
            );
        }
    }

    #[test]
    fn only_the_routes_meant_to_be_public_take_no_credentials() {
        let public: Vec<String> = PATHS
            .iter()
            .filter_map(|path| Some((path, resolve_api_route(path)?)))
            .flat_map(|(path, resolved)| {
                methods(resolved.route)
                    .iter()
                    .filter(|(_, policy)| matches!(policy, AuthPolicy::Public(_)))
                    .map(move |(method, _)| format!("{method} {path}"))
            })
            .collect();
        let mut expected = vec![
            "GET /v2/public/auth/signup",
            "POST /v2/public/auth/signup",
            "POST /v2/public/auth/login",
            "POST /v2/public/auth/logout",
            "POST /v2/public/auth/forgot-password",
            "POST /v2/public/auth/reset-password",
            "GET /v2/public/chat/status",
            "POST /v2/public/chat/demo-token",
            "POST /v2/public/mcp/register",
            "POST /v2/public/mcp/token",
        ];
        if cfg!(feature = "billing") {
            expected.extend([
                "POST /v2/public/webhooks/polar",
                "POST /v2/public/webhooks/stripe",
            ]);
        }
        assert_eq!(public, expected);
    }
}
