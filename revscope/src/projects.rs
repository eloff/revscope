use std::sync::Arc;

use crate::auth::AuthError;
use crate::db::main::Connection;
use crate::db::open_main_database_read;
use crate::db::project::{get_project, is_project_deleted, parse_uuid_key};
use crate::db::{DbError, ProjectCache, insert_project};
use crate::server_context::ServerContext;
use crate::util::block_on;

/// Project names that are readable without authentication (demo dashboards, etc.).
const PUBLIC_PROJECT_NAMES: &[&str] = &["demo"];

pub(crate) fn is_public_project_name(project_name: &str) -> bool {
    PUBLIC_PROJECT_NAMES.contains(&project_name)
}

/// Project names become the ingest URL path (`/{name}`) and `config.project`
/// on the tracker stub. At least 3 characters; only lowercase ASCII letters,
/// digits, and underscores are allowed (matches DB CHECK and tracker).
/// Names of 1-2 characters are reserved for our own root paths, such as the
/// bundled UI at `/ui`.
pub(crate) fn is_valid_project_name(project_name: &str) -> bool {
    project_name.len() >= 3
        && project_name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

/// Strip URL scheme and path/query/fragment, leaving the host-like website value.
pub(crate) fn normalize_website(raw: &str) -> String {
    let trimmed = raw.trim();
    let without_scheme = strip_url_scheme(trimmed);
    without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .to_string()
}

fn strip_url_scheme(value: &str) -> &str {
    if let Some(rest) = value.strip_prefix("//") {
        return rest;
    }
    let Some(scheme_end) = value.find("://") else {
        return value;
    };
    let scheme = &value[..scheme_end];
    let valid_scheme = scheme.bytes().enumerate().all(|(index, byte)| {
        if index == 0 {
            byte.is_ascii_alphabetic()
        } else {
            byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.')
        }
    });
    if valid_scheme && !scheme.is_empty() {
        &value[scheme_end + 3..]
    } else {
        value
    }
}

/// Derive a tracker/ingest project name from a website host: lowercase, then
/// keep only `[a-z0-9_]`.
pub(crate) fn project_name_from_host(account_name: &str) -> String {
    account_name
        .to_ascii_lowercase()
        .bytes()
        .filter(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_')
        .map(char::from)
        .collect()
}

/// The project name a website (or a bare name) becomes: its host, reduced to
/// the characters a project name allows.
pub(crate) fn project_name_from_website(raw: &str) -> String {
    project_name_from_host(&normalize_website(raw))
}

/// Refuses a name a new project can't have, with a message for the form.
/// Mirrored by `validateProjectName` in web/shared/src/projectName.ts.
pub(crate) fn check_new_project_name(name: &str) -> Result<(), AuthError> {
    if !is_valid_project_name(name) {
        return Err(AuthError::InvalidRequest(
            "Project name must be at least 3 lowercase letters, numbers or underscores.",
        ));
    }
    if is_public_project_name(name) {
        return Err(AuthError::InvalidRequest(
            "This project name is reserved. Try a different name.",
        ));
    }
    Ok(())
}

/// The name a new project gets: the one the request sent, which the user may
/// have edited, or else the one its website becomes. A sent name is only
/// trimmed, not reduced, so the project is named exactly what the form showed;
/// a blank one counts as not sent.
pub(crate) fn new_project_name(
    website: &str,
    requested: Option<&str>,
) -> Result<String, AuthError> {
    if let Some(name) = requested.map(str::trim).filter(|name| !name.is_empty()) {
        check_new_project_name(name)?;
        return Ok(name.to_string());
    }
    let name = project_name_from_website(website);
    if !is_valid_project_name(&name) {
        return Err(AuthError::InvalidRequest(
            "Website must contain at least 3 letters or numbers for the project name.",
        ));
    }
    check_new_project_name(&name)?;
    Ok(name)
}

/// Plans that may hold more than one project. Starter and free are sold as
/// "1 website"; keep the plans in sync with `planAllowsMultipleProjects` in
/// web/shared/src/projectSelection.ts and the tiers in web/site pricing.ts.
pub(crate) fn plan_allows_multiple_projects(plan: &str, subscription_status: Option<&str>) -> bool {
    matches!(plan, "growth" | "founder" | "enterprise")
        && plan_includes_paid_features(plan, subscription_status)
}

/// Whether an account's subscription is one it's paying for, which custom domains,
/// team members and extra projects all need. Reads the status as well as the plan because the
/// plan changes when a checkout starts, before any payment, and stays the same
/// after a cancellation. Keep the plans in sync with the tiers in web/site
/// pricing.ts. A plan type added without an entry here doesn't get them.
pub(crate) fn plan_includes_paid_features(plan: &str, subscription_status: Option<&str>) -> bool {
    if cfg!(feature = "self-hosted") {
        return true;
    }
    match plan {
        // Never billed, so it has no subscription status.
        "enterprise" => true,
        "starter" | "growth" | "founder" => {
            matches!(
                subscription_status,
                Some("trialing" | "active" | "past_due")
            )
        }
        _ => false,
    }
}

/// Read fresh for each decision: a session can hold a plan since changed.
pub(crate) async fn project_includes_custom_domain(
    conn: &Connection,
    project_id: i32,
) -> Result<bool, AuthError> {
    let row = conn
        .query_opt(
            "SELECT sub.plan_type::text, sub.subscription_status
             FROM projects p
             JOIN account_subscriptions sub
                ON sub.account_id = p.account_id AND sub.ended_at IS NULL
             WHERE p.id = $1",
            &[&project_id],
        )
        .await
        .map_err(AuthError::from)?
        .ok_or(AuthError::NotFound("no such project"))?;
    Ok(plan_includes_paid_features(row.get(0), row.get(1)))
}

/// Read fresh for each decision: a session can hold a plan since changed.
pub(crate) async fn account_includes_paid_features(
    conn: &Connection,
    account_id: &str,
) -> Result<bool, AuthError> {
    let row = conn
        .query_opt(
            "SELECT plan_type::text, subscription_status
             FROM account_subscriptions
             WHERE account_id = $1::text::uuid AND ended_at IS NULL",
            &[&account_id],
        )
        .await
        .map_err(AuthError::from)?
        .ok_or(AuthError::InvalidSession)?;
    Ok(plan_includes_paid_features(row.get(0), row.get(1)))
}

/// Parse a non-negative project id from an API/query/path string.
/// `0` is allowed for staging (e.g. `generate`); real projects start at `1`.
pub(crate) fn parse_project_id_str(project_id: &str) -> Option<i32> {
    let id: i32 = project_id.parse().ok()?;
    (id >= 0).then_some(id)
}

pub(crate) fn is_valid_project_id(project_id: &str) -> bool {
    parse_project_id_str(project_id).is_some()
}

/// Canonical decimal form (no leading zeros / sign prefix).
pub(crate) fn normalize_project_id(project_id: &str) -> String {
    parse_project_id_str(project_id)
        .map(|id| id.to_string())
        .unwrap_or_else(|| project_id.to_string())
}

/// Load project identity from Postgres (or the in-memory cache) and insert a
/// fully populated [`ProjectCache`].
pub(crate) async fn fetch_project(
    server_context: Arc<ServerContext>,
    project_id: i32,
) -> Result<Option<Arc<ProjectCache>>, DbError> {
    if let Some(project) = get_project(server_context.as_ref(), project_id) {
        return Ok(Some(project));
    }

    let conn = open_main_database_read(server_context.clone()).await?;
    let row = conn
        .query_opt(
            "SELECT name, account_id::text
             FROM projects
             WHERE id = $1",
            &[&project_id],
        )
        .await
        .map_err(DbError::main_postgres_query)?;
    let Some(row) = row else {
        return Ok(None);
    };

    let name: String = row.get(0);
    let account_uuid: String = row.get(1);
    let account_id = parse_uuid_key(&account_uuid).ok_or_else(|| {
        DbError::InvalidProjectId(format!("bad account_id for project {project_id}"))
    })?;
    let project = insert_project(server_context.as_ref(), project_id, name, account_id);
    Ok((!is_project_deleted(server_context.as_ref(), project_id)).then_some(project))
}

/// [`fetch_project`] for sync callers. Cache hits do not require a Tokio runtime;
/// misses use [`block_on`] and must run under one.
pub(crate) fn fetch_project_blocking(
    server_context: Arc<ServerContext>,
    project_id: i32,
) -> Result<Option<Arc<ProjectCache>>, DbError> {
    if let Some(project) = get_project(server_context.as_ref(), project_id) {
        return Ok(Some(project));
    }
    block_on(fetch_project(server_context, project_id))
}

/// Resolve a project name to its shared [`ProjectCache`].
/// Successful mappings are cached for the lifetime of the server context.
pub(crate) async fn resolve_project(
    server_context: Arc<ServerContext>,
    project_name: &str,
) -> Result<Option<Arc<ProjectCache>>, DbError> {
    if !is_valid_project_name(project_name) {
        return Ok(None);
    }

    if let Some(project) = server_context.project_name_cache.get(project_name) {
        project.refresh_db_expiry();
        return Ok(Some(project.clone()));
    }

    let conn = open_main_database_read(server_context.clone()).await?;
    let row = conn
        .query_opt(
            "SELECT id, account_id::text
             FROM projects
             WHERE name = $1",
            &[&project_name],
        )
        .await
        .map_err(DbError::main_postgres_query)?;
    let Some(row) = row else {
        return Ok(None);
    };

    let project_id: i32 = row.get(0);
    if project_id <= 0 {
        return Err(DbError::InvalidProjectId(project_id.to_string()));
    }
    let account_uuid: String = row.get(1);
    let account_id = parse_uuid_key(&account_uuid).ok_or_else(|| {
        DbError::InvalidProjectId(format!("bad account_id for project {project_id}"))
    })?;
    let project = insert_project(
        server_context.as_ref(),
        project_id,
        project_name.to_string(),
        account_id,
    );
    Ok((!is_project_deleted(server_context.as_ref(), project_id)).then_some(project))
}

/// Every project name on an account, alphabetically. Names are what the MCP
/// tools take instead of ids, so this is both the `list_projects` answer and
/// the "did you mean" list on an unknown one.
pub(crate) async fn account_project_names(
    server_context: Arc<ServerContext>,
    account_id: &str,
) -> Result<Vec<String>, DbError> {
    let conn = open_main_database_read(server_context).await?;
    let rows = conn
        .query(
            "SELECT name
             FROM projects
             WHERE account_id = $1::text::uuid
             ORDER BY name",
            &[&account_id],
        )
        .await
        .map_err(DbError::main_postgres_query)?;
    Ok(rows.iter().map(|row| row.get(0)).collect())
}

/// Whether an insert into `projects` failed because another account (or this
/// one) already holds the name. Names are unique across every account.
pub(crate) fn is_project_name_taken(error: &tokio_postgres::Error) -> bool {
    error.code() == Some(&tokio_postgres::error::SqlState::UNIQUE_VIOLATION)
        && crate::auth::db_constraint(error) == Some("projects_name_key")
}

/// Fails when `name` was retired by deleting another account's project; see
/// migrations-main/00018_retired_project_names.sql. Run it after inserting the
/// project: the insert waits on a delete of the same name that hasn't
/// committed, so the check then sees that delete's retirement. Inside a
/// transaction a failure rolls the insert back; a caller outside one must
/// remove the project itself (see hosted signup). The account that retired the
/// name takes it back.
pub(crate) async fn claim_project_name(
    conn: &Connection,
    name: &str,
    account_id: &str,
) -> Result<(), AuthError> {
    let retired_by = conn
        .query_opt(
            "SELECT account_id::text FROM retired_project_names WHERE name = $1",
            &[&name],
        )
        .await?;
    match retired_by {
        None => Ok(()),
        Some(row) if row.get::<_, String>(0) == account_id => {
            conn.execute(
                "DELETE FROM retired_project_names WHERE name = $1",
                &[&name],
            )
            .await?;
            Ok(())
        }
        Some(_) => Err(AuthError::Conflict(PROJECT_NAME_TAKEN)),
    }
}

/// Whether any account has retired `name`, for a caller that can turn a request
/// away before inserting anything. Only hosted signup can.
#[cfg(not(feature = "self-hosted"))]
pub(crate) async fn is_project_name_retired(
    conn: &Connection,
    name: &str,
) -> Result<bool, AuthError> {
    Ok(conn
        .query_opt(
            "SELECT 1 FROM retired_project_names WHERE name = $1",
            &[&name],
        )
        .await?
        .is_some())
}

pub(crate) const PROJECT_NAME_TAKEN: &str =
    "A project with this name already exists. Try another name.";

/// The account as a project create or delete must see it. Read under a lock
/// on the account row, so two such changes racing on one account each count
/// the other's result. Call it inside [`Connection::in_transaction`]; the lock
/// is held until that transaction ends.
pub(crate) struct LockedAccount {
    pub(crate) plan: String,
    pub(crate) subscription_status: Option<String>,
    pub(crate) project_count: i64,
}

pub(crate) async fn lock_account_projects(
    conn: &Connection,
    account_id: &str,
) -> Result<LockedAccount, AuthError> {
    let row = conn
        .query_opt(
            "SELECT sub.plan_type::text, sub.subscription_status
             FROM accounts a
             INNER JOIN account_subscriptions sub
                ON sub.account_id = a.id AND sub.ended_at IS NULL
             WHERE a.id = $1::text::uuid
             FOR UPDATE OF a",
            &[&account_id],
        )
        .await?
        .ok_or(AuthError::InvalidSession)?;
    let project_count: i64 = conn
        .query_one(
            "SELECT COUNT(*) FROM projects WHERE account_id = $1::text::uuid",
            &[&account_id],
        )
        .await?
        .get(0);
    Ok(LockedAccount {
        plan: row.get(0),
        subscription_status: row.get(1),
        project_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(not(feature = "self-hosted"))]
    fn paid_features_start_at_the_starter_plan() {
        for plan in ["starter", "growth", "founder", "enterprise"] {
            assert!(plan_includes_paid_features(plan, Some("active")), "{plan}");
        }
        for plan in ["free", "brand_new_plan", ""] {
            assert!(!plan_includes_paid_features(plan, Some("active")), "{plan}");
        }
    }

    #[test]
    #[cfg(not(feature = "self-hosted"))]
    fn paid_features_need_a_subscription_in_good_standing_on_a_paid_plan() {
        for status in ["trialing", "active", "past_due"] {
            assert!(
                plan_includes_paid_features("starter", Some(status)),
                "{status}"
            );
        }
        for status in [
            None,
            Some("unpaid"),
            Some("incomplete"),
            Some("canceled"),
            Some(""),
        ] {
            assert!(
                !plan_includes_paid_features("starter", status),
                "{status:?}"
            );
        }
        assert!(plan_includes_paid_features("enterprise", None));
    }

    #[test]
    #[cfg(feature = "self-hosted")]
    fn self_hosted_includes_paid_features_whatever_the_plan() {
        assert!(plan_includes_paid_features("free", None));
    }

    #[test]
    fn recognizes_hardcoded_public_project_names() {
        assert!(is_public_project_name("demo"));
        assert!(!is_public_project_name("demo_b"));
        assert!(!is_public_project_name("revscope"));
    }

    #[test]
    fn validates_lowercase_ascii_letters_digits_and_underscores() {
        for valid in [
            "demo",
            "abc",
            "revscope",
            "_private",
            "my_project",
            "123",
            "a1b",
        ] {
            assert!(is_valid_project_name(valid), "{valid}");
        }
        for invalid in [
            "",
            "a",
            "ab",
            "Demo",
            "$internal",
            "with-dash",
            "with space",
            "naïve",
            "dot.name",
        ] {
            assert!(!is_valid_project_name(invalid), "{invalid}");
        }
    }

    #[test]
    fn two_character_names_are_reserved() {
        assert!(!is_valid_project_name("ui"));
        let alphabet = b"abcdefghijklmnopqrstuvwxyz0123456789_";
        for first in alphabet {
            for second in alphabet {
                let name = String::from_utf8(vec![*first, *second]).expect("ascii");
                assert!(!is_valid_project_name(&name), "{name}");
            }
        }
    }

    #[test]
    fn the_website_is_reduced_to_its_host() {
        for (raw, expected) in [
            ("example.com", "example.com"),
            ("  example.com  ", "example.com"),
            ("https://example.com", "example.com"),
            (
                "http://www.example.com/pricing?ref=x#top",
                "www.example.com",
            ),
            ("//example.com/path", "example.com"),
            ("svn+ssh://example.com", "example.com"),
            ("example.com/", "example.com"),
            ("example.com?x=1", "example.com"),
        ] {
            assert_eq!(normalize_website(raw), expected, "{raw}");
        }
    }

    #[test]
    fn the_project_name_keeps_only_lowercase_letters_digits_and_underscores() {
        assert_eq!(project_name_from_host("example.com"), "examplecom");
        assert_eq!(
            project_name_from_host("My-Shop_2.Example.COM"),
            "myshop_2examplecom"
        );
        assert_eq!(project_name_from_host("bücher.de"), "bcherde");
        assert_eq!(project_name_from_host("..."), "");
    }

    #[test]
    fn a_website_or_a_bare_name_becomes_a_project_name() {
        assert_eq!(
            project_name_from_website("https://Shop.Example.com/pricing"),
            "shopexamplecom"
        );
        assert_eq!(project_name_from_website(" my_blog "), "my_blog");
        assert_eq!(project_name_from_website("https://"), "");
    }

    #[test]
    fn only_growth_and_above_hold_more_than_one_project() {
        for plan in ["growth", "founder", "enterprise"] {
            assert!(
                plan_allows_multiple_projects(plan, Some("active")),
                "{plan}"
            );
        }
        for plan in ["free", "starter", ""] {
            assert!(
                !plan_allows_multiple_projects(plan, Some("active")),
                "{plan}"
            );
        }
    }

    #[test]
    #[cfg(not(feature = "self-hosted"))]
    fn choosing_growth_before_paying_does_not_allow_more_projects() {
        for status in [None, Some("incomplete"), Some("canceled")] {
            assert!(
                !plan_allows_multiple_projects("growth", status),
                "{status:?}"
            );
        }
        assert!(plan_allows_multiple_projects("enterprise", None));
    }

    #[test]
    fn parses_non_negative_decimal_project_ids() {
        assert_eq!(parse_project_id_str("0"), Some(0));
        assert_eq!(parse_project_id_str("1"), Some(1));
        assert_eq!(parse_project_id_str("42"), Some(42));
        assert!(parse_project_id_str("-1").is_none());
        assert!(parse_project_id_str("not-a-number").is_none());
        assert!(parse_project_id_str("").is_none());
    }

    #[test]
    fn a_new_project_name_is_the_one_sent_or_else_the_websites() {
        assert_eq!(
            new_project_name("https://Shop.Example.com/", None).unwrap(),
            "shopexamplecom"
        );
        assert_eq!(
            new_project_name("shop.example.com", Some("  ")).unwrap(),
            "shopexamplecom"
        );
        assert_eq!(
            new_project_name("shop.example.com", Some(" my_shop ")).unwrap(),
            "my_shop"
        );
        // A sent name isn't reduced into a valid one.
        assert!(new_project_name("shop.example.com", Some("My-Shop")).is_err());
        assert!(new_project_name("shop.example.com", Some("ab")).is_err());
        assert!(new_project_name("a.b", None).is_err());
        assert!(new_project_name("shop.example.com", Some("demo")).is_err());
        assert!(new_project_name("demo", None).is_err());
    }

    #[test]
    fn caches_project_identity_without_postgres() {
        let context = Arc::new(ServerContext::new_in_memory("project-name-cache"));
        let project_id = 1;
        let account_id = "00000000-0000-0000-0000-000000000001";
        assert!(
            context
                .cache_project(project_id, "invalid-name", account_id)
                .is_none()
        );
        assert!(
            context
                .cache_project(project_id, "ab", account_id)
                .is_none()
        );
        let project = context
            .cache_project(project_id, "demo", account_id)
            .expect("valid project");
        assert_eq!(project.id(), 1);
        assert_eq!(project.name(), "demo");
        assert!(context.project_name_cache.contains_key("demo"));
    }
}
