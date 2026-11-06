//! Authorization grants.
//!
//! The router (`routes::guard`) checks the credentials a route's policy asks
//! for and hands the handler the grant it produced. Handlers pass these
//! grants on to the domain functions they call, and only this module can
//! construct one, so a domain function that demands a grant can't be reached
//! by code that skipped the check.

use std::net::IpAddr;
use std::ops::Deref;
use std::sync::Arc;

use crate::auth::{self, AuthError, SessionCookie, SessionUser};
use crate::db::{ProjectCache, parse_project_id};
use crate::download_db::DEMO_PROJECT_ID;
use crate::projects::is_public_project_name;
use crate::query_token::{ProjectAuth, authorize_project as authorize_project_credential};
use crate::server_context::ServerContext;
use crate::tokens::TokenSecret;
use crate::user_access::{require_admin, user_project_access};
use crate::util::block_on;

/// Proof that the request's session is live: Postgres (or the session cache,
/// which only holds sessions Postgres confirmed) knows it and it is within its
/// max age.
#[derive(Debug, Clone)]
pub(crate) struct Account {
    user: SessionUser,
    session: SessionCookie,
}

impl Account {
    pub(crate) fn user(&self) -> &SessionUser {
        &self.user
    }

    pub(crate) fn session(&self) -> &SessionCookie {
        &self.session
    }
}

/// Proof that the signed-in user is an admin of their account, for billing,
/// managing users, and adding projects. Read from the session, which another
/// process may serve for its cache TTL after a demotion; changes to users
/// re-check the role in Postgres under the account lock as well.
#[derive(Debug, Clone)]
pub(crate) struct AdminAccount {
    account: Account,
}

impl Deref for AdminAccount {
    type Target = Account;

    fn deref(&self) -> &Account {
        &self.account
    }
}

/// Proof that the current request may read this project. Constructed only by
/// the helpers in this module.
#[derive(Debug, Clone)]
pub(crate) struct AuthorizedProject {
    id: String,
    key: i32,
}

impl AuthorizedProject {
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    pub(crate) fn key(&self) -> i32 {
        self.key
    }

    fn from_parsed(project_id: &str) -> Result<Self, AuthError> {
        let key = parse_project_id(project_id)?;
        Ok(Self {
            id: crate::projects::normalize_project_id(project_id),
            key,
        })
    }
}

/// Proof that the signed-in caller's account owns this project, so never
/// the demo for anyone but its owner. Derefs to the read grant; a read grant
/// can never become one of these.
#[derive(Debug, Clone)]
pub(crate) struct OwnedProject {
    project: AuthorizedProject,
}

impl Deref for OwnedProject {
    type Target = AuthorizedProject;

    fn deref(&self) -> &AuthorizedProject {
        &self.project
    }
}

/// Proof that the caller may download a copy of this project's whole
/// database: its owner, any signed-in account for the demo, or a download
/// token. Distinct from the read grant, so a download can't be
/// served under the read policy's anonymous demo access.
#[derive(Debug, Clone)]
pub(crate) struct DownloadableProject {
    project: AuthorizedProject,
}

impl Deref for DownloadableProject {
    type Target = AuthorizedProject;

    fn deref(&self) -> &AuthorizedProject {
        &self.project
    }
}

/// Proof that the signed-in owner may change this project, which is never the
/// public demo. Derefs to the owner's grant, since whoever may write may read.
#[derive(Debug, Clone)]
pub(crate) struct WritableProject {
    project: OwnedProject,
}

impl Deref for WritableProject {
    type Target = OwnedProject;

    fn deref(&self) -> &OwnedProject {
        &self.project
    }
}

/// Proof that an admin may change this project's settings or delete it, which
/// members may not even on projects they can read.
#[derive(Debug, Clone)]
pub(crate) struct AdminProject {
    admin: AdminAccount,
    project: WritableProject,
}

impl AdminProject {
    pub(crate) fn admin(&self) -> &AdminAccount {
        &self.admin
    }
}

impl Deref for AdminProject {
    type Target = WritableProject;

    fn deref(&self) -> &WritableProject {
        &self.project
    }
}

/// Who is asking in Ask: a signed-in account, or an anonymous demo visitor.
#[derive(Debug, Clone)]
pub(crate) enum ChatCaller {
    SignedIn(SessionUser),
    DemoGuest { client_ip: IpAddr },
}

/// Proof that `caller` may use Ask on `project`.
#[derive(Debug, Clone)]
pub(crate) struct ChatGrant {
    project: AuthorizedProject,
    caller: ChatCaller,
}

impl ChatGrant {
    pub(crate) fn project(&self) -> &AuthorizedProject {
        &self.project
    }

    pub(crate) fn caller(&self) -> &ChatCaller {
        &self.caller
    }
}

/// Verifies a session against Postgres or the session cache.
pub(crate) fn authorize_account(
    server_context: Arc<ServerContext>,
    session: &SessionCookie,
) -> Result<Account, AuthError> {
    let user = block_on(auth::session_user(server_context, session))?;
    Ok(Account {
        user,
        session: session.clone(),
    })
}

/// An admin of the account the session belongs to.
pub(crate) fn authorize_admin(account: &Account) -> Result<AdminAccount, AuthError> {
    require_admin(&account.user)?;
    Ok(AdminAccount {
        account: account.clone(),
    })
}

/// An admin's own project, blocked for the public demo.
pub(crate) fn authorize_admin_project(
    server_context: Arc<ServerContext>,
    admin: &AdminAccount,
    project_id: &str,
) -> Result<AdminProject, AuthError> {
    Ok(AdminProject {
        admin: admin.clone(),
        project: authorize_session_project_mut(server_context, &admin.account, project_id)?,
    })
}

/// Own projects the user may read only. Used by `/v2/session/projects/{id}/…`.
pub(crate) fn authorize_session_project(
    server_context: Arc<ServerContext>,
    account: &Account,
    project_id: &str,
) -> Result<OwnedProject, AuthError> {
    ensure_project_access(server_context, &account.user, project_id)?;
    Ok(OwnedProject {
        project: AuthorizedProject::from_parsed(project_id)?,
    })
}

/// Own projects, or the public demo for any signed-in account. Used by the
/// database download routes: the demo's data is public, but a download copies
/// the whole database, so it takes a session rather than none at all. Matches
/// the demo by id, not by [`is_public_project_name`], so a project later made
/// publicly readable does not also become downloadable by every account.
pub(crate) fn authorize_session_project_or_demo(
    server_context: Arc<ServerContext>,
    account: &Account,
    project_id: &str,
) -> Result<DownloadableProject, AuthError> {
    match ensure_project_access(server_context.clone(), &account.user, project_id) {
        Ok(()) => {}
        Err(AuthError::InvalidCredentials) if is_demo_project_id(project_id) => {
            ensure_project_is_public(server_context, project_id)?;
        }
        Err(error) => return Err(error),
    }
    Ok(DownloadableProject {
        project: AuthorizedProject::from_parsed(project_id)?,
    })
}

/// Own projects, blocked for the public demo. Used by mutating session routes.
pub(crate) fn authorize_session_project_mut(
    server_context: Arc<ServerContext>,
    account: &Account,
    project_id: &str,
) -> Result<WritableProject, AuthError> {
    let project = authorize_session_project(server_context.clone(), account, project_id)?;
    ensure_project_not_public(server_context, project.id())?;
    Ok(WritableProject { project })
}

/// Own project when signed in, otherwise the public demo only.
/// Used by `/v2/read/projects/{id}/…`.
pub(crate) fn authorize_read_project(
    server_context: Arc<ServerContext>,
    account: Option<&Account>,
    project_id: &str,
) -> Result<AuthorizedProject, AuthError> {
    if let Some(account) = account {
        ensure_project_access_or_public(server_context, &account.user, project_id)?;
    } else {
        ensure_project_is_public(server_context, project_id)?;
    }
    AuthorizedProject::from_parsed(project_id)
}

/// Session or project-scoped query token. Used by `/v2/query/projects/{id}/…`.
pub(crate) fn authorize_query_project(
    server_context: Arc<ServerContext>,
    auth: &ProjectAuth,
    project_id: &str,
) -> Result<AuthorizedProject, AuthError> {
    authorize_project_credential(server_context, auth, project_id)?;
    AuthorizedProject::from_parsed(project_id)
}

/// A token minted by `download_db::create_download_token`, which grants
/// downloading one project's database and nothing else. Used by
/// `/v2/download/projects/{id}` when the browser navigates without a session.
pub(crate) fn authorize_download_token(
    server_context: Arc<ServerContext>,
    token: &str,
    project_id: &str,
) -> Result<DownloadableProject, AuthError> {
    let project_key = parse_project_id(project_id)?;
    let secret = TokenSecret::parse(token).ok_or(AuthError::InvalidSession)?;
    let granted = server_context
        .download_tokens
        .get(secret)
        .ok_or(AuthError::InvalidSession)?;
    if granted.project_key != project_key {
        return Err(AuthError::InvalidSession);
    }
    crate::db::require_project(server_context.clone(), project_key)
        .map_err(|error| AuthError::Db(error.to_string()))?;
    // The token outlives the session cache and lives only in this process, so
    // an admin removing the holder or narrowing their projects elsewhere has
    // to be read from Postgres. The demo needs no holder; it is matched by
    // id, as for session downloads, so another project named like it does
    // not skip the check.
    if !is_demo_project_id(project_id) {
        block_on(async {
            let conn = crate::db::open_main_database_read(server_context).await?;
            user_project_access(
                &conn,
                &granted.user_id,
                &granted.account_id,
                Some(project_key),
            )
            .await
        })
        .map_err(|error| match error {
            AuthError::InvalidCredentials => AuthError::InvalidSession,
            error => error,
        })?;
    }
    Ok(DownloadableProject {
        project: AuthorizedProject::from_parsed(project_id)?,
    })
}

/// Ask on an own project, or on the public demo by any signed-in account or,
/// when a client IP identifies them, an anonymous visitor. Used by
/// `/v2/chat/projects/{id}/…`.
pub(crate) fn authorize_chat(
    server_context: Arc<ServerContext>,
    account: Option<&Account>,
    project_id: &str,
    client_ip: Option<IpAddr>,
) -> Result<ChatGrant, AuthError> {
    let project = authorize_read_project(server_context, account, project_id)?;
    let caller = match account {
        Some(account) => ChatCaller::SignedIn(account.user.clone()),
        // A publicly named project other than the demo stays owner-only:
        // anonymous Ask is billed to, and stored with, the demo alone.
        None if !is_demo_project_id(project.id()) => return Err(AuthError::InvalidSession),
        None => {
            let client_ip = client_ip.ok_or(AuthError::InvalidRequest(
                "a client IP is required for anonymous demo Ask",
            ))?;
            ChatCaller::DemoGuest { client_ip }
        }
    };
    Ok(ChatGrant { project, caller })
}

/// A project named by a tool call on an account-scoped MCP token. The token
/// grants every project on `account_key` and nothing else, so ownership is the
/// whole check; [`crate::mcp::token`] has already verified the token is live.
pub(crate) fn authorize_mcp_project(
    account_key: u128,
    project: &ProjectCache,
) -> Result<AuthorizedProject, AuthError> {
    if project.account_id() != account_key {
        return Err(AuthError::InvalidCredentials);
    }
    AuthorizedProject::from_parsed(&project.id_string())
}

fn is_demo_project_id(project_id: &str) -> bool {
    parse_project_id(project_id).ok() == parse_project_id(DEMO_PROJECT_ID).ok()
}

pub(crate) fn ensure_project_is_public(
    server_context: Arc<ServerContext>,
    project_id: &str,
) -> Result<(), AuthError> {
    let project_key = parse_project_id(project_id)?;
    let project = crate::projects::fetch_project_blocking(server_context, project_key)?
        .ok_or(AuthError::InvalidCredentials)?;
    if is_public_project_name(project.name()) {
        Ok(())
    } else {
        Err(AuthError::InvalidCredentials)
    }
}

pub(crate) fn ensure_project_access(
    server_context: Arc<ServerContext>,
    user: &SessionUser,
    project_id: &str,
) -> Result<(), AuthError> {
    let project_key = parse_project_id(project_id)?;
    let user_account = crate::db::project::parse_uuid_key(&user.account_id)
        .ok_or(AuthError::InvalidCredentials)?;
    let project = crate::projects::fetch_project_blocking(server_context, project_key)?
        .ok_or(AuthError::InvalidCredentials)?;
    if project.account_id() == user_account && user.projects.allows(project_key) {
        Ok(())
    } else {
        Err(AuthError::InvalidCredentials)
    }
}

/// Own projects, or the public demo.
pub(crate) fn ensure_project_access_or_public(
    server_context: Arc<ServerContext>,
    user: &SessionUser,
    project_id: &str,
) -> Result<(), AuthError> {
    match ensure_project_access(server_context.clone(), user, project_id) {
        Ok(()) => Ok(()),
        Err(AuthError::InvalidCredentials) => ensure_project_is_public(server_context, project_id),
        Err(error) => Err(error),
    }
}

/// Blocks mutations against the reserved public demo project.
pub(crate) fn ensure_project_not_public(
    server_context: Arc<ServerContext>,
    project_id: &str,
) -> Result<(), AuthError> {
    match ensure_project_is_public(server_context, project_id) {
        Ok(()) => Err(AuthError::InvalidRequest("the demo project is read-only")),
        Err(AuthError::InvalidCredentials) => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    //! Every grant here is checked against caches seeded in memory, so no
    //! Postgres is needed: sessions, projects and query tokens are all served
    //! from [`ServerContext`] before any database lookup.

    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::tokens::TokenSecret;
    use crate::user_access::ProjectScope;
    use crate::uuid_token::UuidToken;

    const DEMO_ID: i32 = 1;
    const OWN_ID: i32 = 7;
    const OTHER_ID: i32 = 8;
    const DEMO_ACCOUNT: &str = "00000000-0000-0000-0000-0000000000a1";
    const OWN_ACCOUNT: &str = "00000000-0000-0000-0000-0000000000b1";
    const OTHER_ACCOUNT: &str = "00000000-0000-0000-0000-0000000000c1";

    static SESSION_COUNTER: AtomicU64 = AtomicU64::new(1);

    /// Holds the runtime that [`block_on`] needs, entered for the test's
    /// lifetime, and a context with the demo and two accounts' projects.
    struct Fixture {
        context: Arc<ServerContext>,
        runtime: tokio::runtime::Runtime,
    }

    impl Fixture {
        fn new() -> Self {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .expect("tokio runtime");
            let unique = SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
            let context = Arc::new(ServerContext::new_in_memory(format!(
                "authorize-unit-{}-{unique}",
                std::process::id()
            )));
            for (id, name, account) in [
                (DEMO_ID, "demo", DEMO_ACCOUNT),
                (OWN_ID, "ownsite", OWN_ACCOUNT),
                (OTHER_ID, "othersite", OTHER_ACCOUNT),
            ] {
                assert!(context.cache_project(id, name, account).is_some());
            }
            Self { context, runtime }
        }

        fn ctx(&self) -> Arc<ServerContext> {
            self.context.clone()
        }

        /// A live session for a user on `account_id`, cached as a Postgres
        /// lookup would have cached it.
        fn session_for(&self, account_id: &str) -> SessionCookie {
            let token = UuidToken::generate().expect("generate");
            let cache = &self.context.session_cache;
            cache.insert(
                token.key(),
                user_on(account_id),
                u32::MAX,
                cache.generation(),
            );
            SessionCookie {
                session_id: token.to_string(),
            }
        }

        /// A verified account for a user on `account_id`.
        fn account_for(&self, account_id: &str) -> Account {
            let session = self.session_for(account_id);
            self.run(|| authorize_account(self.ctx(), &session))
                .expect("live session")
        }

        /// A verified account for a user on `account_id` with `scope` and role.
        fn account_with(&self, account_id: &str, is_admin: bool, scope: ProjectScope) -> Account {
            let token = UuidToken::generate().expect("generate");
            let mut user = user_on(account_id);
            user.is_admin = is_admin;
            user.projects = scope;
            let cache = &self.context.session_cache;
            cache.insert(token.key(), user, u32::MAX, cache.generation());
            let session = SessionCookie {
                session_id: token.to_string(),
            };
            self.run(|| authorize_account(self.ctx(), &session))
                .expect("live session")
        }

        /// A UUIDv7 cookie, the form sessions took before only their hashes
        /// were stored. Cached as live under its bits, so only the check that
        /// it is a token we issue can refuse it.
        fn unissuable_session_for(&self, account_id: &str) -> SessionCookie {
            let session_id = "0198f0a1-2b3c-7d4e-8f90-1a2b3c4d5e6f".to_string();
            let key = u128::from_str_radix(&session_id.replace('-', ""), 16).expect("hex");
            let cache = &self.context.session_cache;
            cache.insert(key, user_on(account_id), u32::MAX, cache.generation());
            SessionCookie { session_id }
        }

        fn query_token_for(&self, project_key: i32) -> TokenSecret {
            self.context
                .query_token_cache
                .issue(project_key, 600)
                .expect("issue query token")
                .secret
        }

        /// Runs `f` with the runtime entered but outside any task, as route
        /// handlers run, since [`block_on`] panics inside an async context.
        fn run<T>(&self, f: impl FnOnce() -> T) -> T {
            let _guard = self.runtime.enter();
            f()
        }
    }

    fn user_on(account_id: &str) -> SessionUser {
        SessionUser {
            user_id: format!("user-of-{account_id}"),
            account_id: account_id.to_string(),
            name: "Test".to_string(),
            email: "test@example.com".to_string(),
            plan: "starter".to_string(),
            is_admin: false,
            projects: ProjectScope::All,
        }
    }

    fn assert_rejected<T: std::fmt::Debug>(result: Result<T, AuthError>) {
        assert!(
            matches!(result, Err(AuthError::InvalidCredentials)),
            "expected InvalidCredentials, got {result:?}"
        );
    }

    fn assert_grants(result: Result<AuthorizedProject, AuthError>, project_key: i32) {
        let project = result.expect("access granted");
        assert_eq!(project.key(), project_key);
        assert_eq!(project.id(), project_key.to_string());
    }

    #[test]
    fn a_user_limited_to_some_projects_reaches_only_those() {
        const SECOND_OWN_ID: i32 = 10;
        let fx = Fixture::new();
        assert!(
            fx.context
                .cache_project(SECOND_OWN_ID, "ownshop", OWN_ACCOUNT)
                .is_some()
        );
        let limited = fx.account_with(
            OWN_ACCOUNT,
            false,
            ProjectScope::from_ids(Some(vec![SECOND_OWN_ID])),
        );
        let query = ProjectAuth::Session(limited.session().clone());
        fx.run(|| {
            assert_grants(
                authorize_session_project(fx.ctx(), &limited, &SECOND_OWN_ID.to_string())
                    .map(|project| project.deref().clone()),
                SECOND_OWN_ID,
            );
            assert_rejected(authorize_session_project(
                fx.ctx(),
                &limited,
                &OWN_ID.to_string(),
            ));
            assert_rejected(authorize_session_project_mut(
                fx.ctx(),
                &limited,
                &OWN_ID.to_string(),
            ));
            assert_rejected(authorize_read_project(
                fx.ctx(),
                Some(&limited),
                &OWN_ID.to_string(),
            ));
            assert_rejected(authorize_query_project(
                fx.ctx(),
                &query,
                &OWN_ID.to_string(),
            ));
            assert_rejected(authorize_session_project_or_demo(
                fx.ctx(),
                &limited,
                &OWN_ID.to_string(),
            ));
            // The public demo stays readable, as it is for any signed-in user.
            assert_grants(
                authorize_read_project(fx.ctx(), Some(&limited), &DEMO_ID.to_string()),
                DEMO_ID,
            );
        });
    }

    #[test]
    fn only_admins_get_the_admin_grants() {
        let fx = Fixture::new();
        let member = fx.account_with(OWN_ACCOUNT, false, ProjectScope::All);
        let admin = fx.account_with(OWN_ACCOUNT, true, ProjectScope::All);
        assert!(matches!(
            authorize_admin(&member),
            Err(AuthError::Forbidden(_))
        ));
        let admin = authorize_admin(&admin).expect("admin");
        fx.run(|| {
            assert_grants(
                authorize_admin_project(fx.ctx(), &admin, &OWN_ID.to_string())
                    .map(|project| (***project).clone()),
                OWN_ID,
            );
            assert_rejected(authorize_admin_project(
                fx.ctx(),
                &admin,
                &OTHER_ID.to_string(),
            ));
        });
    }

    #[test]
    fn session_project_admits_only_the_callers_own_account() {
        let fx = Fixture::new();
        let own = fx.account_for(OWN_ACCOUNT);
        fx.run(|| {
            assert_grants(
                authorize_session_project(fx.ctx(), &own, &OWN_ID.to_string())
                    .map(|project| project.deref().clone()),
                OWN_ID,
            );
            assert_rejected(authorize_session_project(
                fx.ctx(),
                &own,
                &OTHER_ID.to_string(),
            ));
            // Session routes manage a project, so the demo is not one of yours.
            assert_rejected(authorize_session_project(
                fx.ctx(),
                &own,
                &DEMO_ID.to_string(),
            ));
        });
    }

    #[test]
    fn a_padded_project_id_is_checked_and_granted_as_the_same_project() {
        let fx = Fixture::new();
        let own = fx.account_for(OWN_ACCOUNT);
        fx.run(|| {
            // The grant must name the project that was checked, not a second
            // parse of the raw id that could disagree with it.
            assert_grants(
                authorize_session_project(fx.ctx(), &own, "007")
                    .map(|project| project.deref().clone()),
                OWN_ID,
            );
            assert_rejected(authorize_session_project(fx.ctx(), &own, "008"));
            assert_rejected(authorize_read_project(fx.ctx(), Some(&own), "+8"));
        });
    }

    #[test]
    fn a_malformed_project_id_is_never_granted() {
        let fx = Fixture::new();
        let own = fx.account_for(OWN_ACCOUNT);
        fx.run(|| {
            for id in ["", "-7", "seven", "7.0", "99999999999"] {
                assert!(
                    authorize_session_project(fx.ctx(), &own, id).is_err(),
                    "session grant for {id:?}"
                );
                assert!(
                    authorize_read_project(fx.ctx(), None, id).is_err(),
                    "anonymous read grant for {id:?}"
                );
            }
        });
    }

    #[test]
    fn an_unissuable_session_is_refused_before_any_project_check() {
        let fx = Fixture::new();
        let unissuable = fx.unissuable_session_for(OWN_ACCOUNT);
        fx.run(|| {
            // Every session-backed grant needs an `Account` first. That the
            // router refuses, rather than demotes to anonymous, a dead cookie
            // on the demo is covered by `tests/authorization_e2e.rs`.
            let result = authorize_account(fx.ctx(), &unissuable);
            assert!(
                matches!(result, Err(AuthError::InvalidSession)),
                "expected InvalidSession, got {result:?}"
            );
        });
    }

    #[test]
    fn session_project_mut_admits_own_projects_but_never_the_demo() {
        let fx = Fixture::new();
        let own = fx.account_for(OWN_ACCOUNT);
        let demo_owner = fx.account_for(DEMO_ACCOUNT);
        fx.run(|| {
            assert_grants(
                authorize_session_project_mut(fx.ctx(), &own, &OWN_ID.to_string())
                    .map(|project| (**project).clone()),
                OWN_ID,
            );
            assert_rejected(authorize_session_project_mut(
                fx.ctx(),
                &own,
                &OTHER_ID.to_string(),
            ));
            // Even the account that owns the demo may not change it.
            let result = authorize_session_project_mut(fx.ctx(), &demo_owner, &DEMO_ID.to_string());
            assert!(
                matches!(
                    result,
                    Err(AuthError::InvalidRequest("the demo project is read-only"))
                ),
                "got {result:?}"
            );
        });
    }

    #[test]
    fn session_downloads_reach_own_projects_and_the_demo_only() {
        let fx = Fixture::new();
        let own = fx.account_for(OWN_ACCOUNT);
        fx.run(|| {
            assert_grants(
                authorize_session_project_or_demo(fx.ctx(), &own, &OWN_ID.to_string())
                    .map(|project| project.deref().clone()),
                OWN_ID,
            );
            assert_grants(
                authorize_session_project_or_demo(fx.ctx(), &own, &DEMO_ID.to_string())
                    .map(|project| project.deref().clone()),
                DEMO_ID,
            );
            assert_rejected(authorize_session_project_or_demo(
                fx.ctx(),
                &own,
                &OTHER_ID.to_string(),
            ));
        });
    }

    #[test]
    fn session_downloads_do_not_reach_other_publicly_named_projects() {
        let fx = Fixture::new();
        let own = fx.account_for(OWN_ACCOUNT);
        // Readable by anyone, as the demo is, but not the demo.
        const PUBLIC_ID: i32 = 9;
        assert!(
            fx.context
                .cache_project(PUBLIC_ID, "demo", OTHER_ACCOUNT)
                .is_some()
        );
        fx.run(|| {
            assert_grants(
                authorize_read_project(fx.ctx(), Some(&own), &PUBLIC_ID.to_string()),
                PUBLIC_ID,
            );
            assert_rejected(authorize_session_project_or_demo(
                fx.ctx(),
                &own,
                &PUBLIC_ID.to_string(),
            ));
            assert_grants(
                authorize_session_project_or_demo(fx.ctx(), &own, "001")
                    .map(|project| project.deref().clone()),
                DEMO_ID,
            );
        });
    }

    #[test]
    fn anonymous_reads_reach_the_demo_and_nothing_else() {
        let fx = Fixture::new();
        fx.run(|| {
            assert_grants(
                authorize_read_project(fx.ctx(), None, &DEMO_ID.to_string()),
                DEMO_ID,
            );
            assert_rejected(authorize_read_project(fx.ctx(), None, &OWN_ID.to_string()));
            assert_rejected(authorize_read_project(
                fx.ctx(),
                None,
                &OTHER_ID.to_string(),
            ));
        });
    }

    #[test]
    fn signed_in_reads_reach_own_projects_and_the_demo_only() {
        let fx = Fixture::new();
        let own = fx.account_for(OWN_ACCOUNT);
        fx.run(|| {
            assert_grants(
                authorize_read_project(fx.ctx(), Some(&own), &OWN_ID.to_string()),
                OWN_ID,
            );
            assert_grants(
                authorize_read_project(fx.ctx(), Some(&own), &DEMO_ID.to_string()),
                DEMO_ID,
            );
            assert_rejected(authorize_read_project(
                fx.ctx(),
                Some(&own),
                &OTHER_ID.to_string(),
            ));
        });
    }

    #[test]
    fn a_session_user_with_an_unparseable_account_reaches_nothing() {
        let fx = Fixture::new();
        let broken = fx.account_for("not-a-uuid");
        fx.run(|| {
            assert_rejected(authorize_session_project(
                fx.ctx(),
                &broken,
                &OWN_ID.to_string(),
            ));
            assert_rejected(ensure_project_access(
                fx.ctx(),
                &user_on("not-a-uuid"),
                &OWN_ID.to_string(),
            ));
        });
    }

    #[test]
    fn query_sessions_follow_account_ownership() {
        let fx = Fixture::new();
        let own = ProjectAuth::Session(fx.session_for(OWN_ACCOUNT));
        fx.run(|| {
            assert_grants(
                authorize_query_project(fx.ctx(), &own, &OWN_ID.to_string()),
                OWN_ID,
            );
            assert_rejected(authorize_query_project(
                fx.ctx(),
                &own,
                &OTHER_ID.to_string(),
            ));
            // Unlike reads, the query policy has no public-demo fallback.
            assert_rejected(authorize_query_project(
                fx.ctx(),
                &own,
                &DEMO_ID.to_string(),
            ));
        });
    }

    #[test]
    fn a_query_token_reaches_only_the_project_it_was_minted_for() {
        let fx = Fixture::new();
        let token = ProjectAuth::QueryToken(fx.query_token_for(OWN_ID));
        fx.run(|| {
            assert_grants(
                authorize_query_project(fx.ctx(), &token, &OWN_ID.to_string()),
                OWN_ID,
            );
            assert_grants(authorize_query_project(fx.ctx(), &token, "007"), OWN_ID);
            for other in [OTHER_ID, DEMO_ID] {
                assert_rejected(authorize_query_project(
                    fx.ctx(),
                    &token,
                    &other.to_string(),
                ));
            }
        });
    }

    #[test]
    fn mcp_grants_follow_the_token_account() {
        let fx = Fixture::new();
        let own_key = crate::db::project::parse_uuid_key(OWN_ACCOUNT).expect("uuid");
        let own_project = fx.context.cache_project(OWN_ID, "ownsite", OWN_ACCOUNT);
        let other_project = fx
            .context
            .cache_project(OTHER_ID, "othersite", OTHER_ACCOUNT);
        let demo_project = fx.context.cache_project(DEMO_ID, "demo", DEMO_ACCOUNT);
        assert_grants(
            authorize_mcp_project(own_key, &own_project.expect("own project")),
            OWN_ID,
        );
        assert_rejected(authorize_mcp_project(
            own_key,
            &other_project.expect("other project"),
        ));
        assert_rejected(authorize_mcp_project(
            own_key,
            &demo_project.expect("demo project"),
        ));
    }

    #[test]
    fn a_download_token_reaches_only_the_project_it_was_minted_for() {
        let fx = Fixture::new();
        let issued = fx
            .context
            .download_tokens
            .issue(
                crate::download_db::DownloadGrant {
                    project_key: OWN_ID,
                    user_id: user_on(OWN_ACCOUNT).user_id,
                    account_id: OWN_ACCOUNT.to_string(),
                },
                600,
            )
            .expect("issue download token");
        let token = issued.secret.to_string();
        fx.run(|| {
            for project in [OTHER_ID, DEMO_ID] {
                let result = authorize_download_token(fx.ctx(), &token, &project.to_string());
                assert!(
                    matches!(result, Err(AuthError::InvalidSession)),
                    "download token reached project {project}: {result:?}"
                );
            }
            let result = authorize_download_token(fx.ctx(), "not-a-token", &OWN_ID.to_string());
            assert!(
                matches!(result, Err(AuthError::InvalidSession)),
                "{result:?}"
            );
        });
    }

    #[test]
    fn anonymous_ask_reaches_the_demo_only_and_needs_a_client_ip() {
        let fx = Fixture::new();
        let ip: IpAddr = "203.0.113.7".parse().expect("ip");
        const PUBLIC_ID: i32 = 9;
        assert!(
            fx.context
                .cache_project(PUBLIC_ID, "demo", OTHER_ACCOUNT)
                .is_some()
        );
        fx.run(|| {
            let grant = authorize_chat(fx.ctx(), None, &DEMO_ID.to_string(), Some(ip))
                .expect("anonymous demo Ask");
            assert_eq!(grant.project().key(), DEMO_ID);
            assert!(
                matches!(grant.caller(), ChatCaller::DemoGuest { client_ip } if *client_ip == ip)
            );
            assert!(matches!(
                authorize_chat(fx.ctx(), None, &DEMO_ID.to_string(), None),
                Err(AuthError::InvalidRequest(_))
            ));
            // Readable by anyone, but anonymous Ask is the demo's alone.
            assert!(authorize_chat(fx.ctx(), None, &PUBLIC_ID.to_string(), Some(ip)).is_err());
            assert_rejected(authorize_chat(
                fx.ctx(),
                None,
                &OWN_ID.to_string(),
                Some(ip),
            ));
        });
    }

    #[test]
    fn signed_in_ask_reaches_own_projects_and_the_demo_only() {
        let fx = Fixture::new();
        let own = fx.account_for(OWN_ACCOUNT);
        fx.run(|| {
            for project in [OWN_ID, DEMO_ID] {
                let grant = authorize_chat(fx.ctx(), Some(&own), &project.to_string(), None)
                    .expect("signed-in Ask");
                assert_eq!(grant.project().key(), project);
                assert!(matches!(grant.caller(), ChatCaller::SignedIn(user) if user.account_id == OWN_ACCOUNT));
            }
            assert_rejected(authorize_chat(fx.ctx(), Some(&own), &OTHER_ID.to_string(), None));
        });
    }

    #[test]
    fn only_the_demo_counts_as_public() {
        let fx = Fixture::new();
        fx.run(|| {
            assert!(ensure_project_is_public(fx.ctx(), &DEMO_ID.to_string()).is_ok());
            assert_rejected(ensure_project_is_public(fx.ctx(), &OWN_ID.to_string()));
            assert!(ensure_project_not_public(fx.ctx(), &OWN_ID.to_string()).is_ok());
            assert!(ensure_project_not_public(fx.ctx(), &DEMO_ID.to_string()).is_err());
        });
    }
}
