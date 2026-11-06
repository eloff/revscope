//! Who may do what on an account. Admins see and change billing, add and
//! delete projects, change project settings and manage users. Everyone else
//! uses the dashboards, Ask, funnels and campaigns, on every project or only
//! the ones an admin picked for them.
//!
//! The session cache serves a user's access for up to
//! [`crate::session_cache::SESSION_CACHE_TTL_SECONDS`] on other processes, so
//! every change to users re-reads the acting admin from Postgres under a lock
//! on the account row rather than trusting their session.

use std::sync::Arc;

use serde::Deserialize;

use crate::auth::{AuthError, SessionUser, map_query_error};
use crate::db::main::Connection;
use crate::server_context::ServerContext;

pub(crate) const ADMIN_ONLY: &str = "Only an account admin can do this.";

/// The projects on their account a user may read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProjectScope {
    All,
    /// Sorted. May be empty once an admin deletes every project picked.
    Only(Vec<i32>),
}

impl ProjectScope {
    /// `None` means every project, as `load_session_user` selects it.
    pub(crate) fn from_ids(project_ids: Option<Vec<i32>>) -> Self {
        match project_ids {
            None => Self::All,
            Some(mut ids) => {
                ids.sort_unstable();
                Self::Only(ids)
            }
        }
    }

    pub(crate) fn allows(&self, project_key: i32) -> bool {
        match self {
            Self::All => true,
            Self::Only(ids) => ids.binary_search(&project_key).is_ok(),
        }
    }

    /// For binding as an `int4[]` that is NULL for every project.
    pub(crate) fn project_ids(&self) -> Option<&[i32]> {
        match self {
            Self::All => None,
            Self::Only(ids) => Some(ids),
        }
    }
}

pub(crate) fn require_admin(user: &SessionUser) -> Result<(), AuthError> {
    if user.is_admin {
        Ok(())
    } else {
        Err(AuthError::Forbidden(ADMIN_ONLY))
    }
}

/// Access as an admin asks for it when inviting or editing a user. Both
/// fields are required, so a request that leaves one out can neither demote
/// an admin nor widen a member to every project.
#[derive(Debug, Deserialize)]
pub(crate) struct AccessRequest {
    pub(crate) is_admin: bool,
    /// Null for every project, now and later. Ignored for admins, who always
    /// have every project.
    #[serde(deserialize_with = "required_nullable")]
    pub(crate) project_ids: Option<Vec<i32>>,
}

/// Serde reads a missing `Option` field as `None` unless the field has its
/// own deserializer, so this one exists only to make the field required.
fn required_nullable<'de, D>(deserializer: D) -> Result<Option<Vec<i32>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::deserialize(deserializer)
}

/// An [`AccessRequest`] with its project list sorted, deduplicated, and
/// dropped for admins. Not yet checked against the account's projects; see
/// [`write_user_access`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UserAccess {
    pub(crate) is_admin: bool,
    pub(crate) projects: ProjectScope,
}

impl UserAccess {
    pub(crate) fn from_request(request: AccessRequest) -> Result<Self, AuthError> {
        let projects = match request.project_ids {
            _ if request.is_admin => ProjectScope::All,
            None => ProjectScope::All,
            Some(mut ids) => {
                ids.sort_unstable();
                ids.dedup();
                if ids.is_empty() {
                    return Err(AuthError::InvalidRequest(
                        "choose at least one project, or give access to all of them",
                    ));
                }
                ProjectScope::Only(ids)
            }
        };
        Ok(Self {
            is_admin: request.is_admin,
            projects,
        })
    }
}

/// Locks the account row, then confirms `actor_user_id` is still an admin on
/// it. Call inside [`Connection::in_transaction`]. Holding the lock is what
/// keeps two admins demoting or removing each other at once from leaving the
/// account with none: whichever runs second finds it is no longer an admin.
pub(crate) async fn lock_account_as_admin(
    conn: &Connection,
    account_id: &str,
    actor_user_id: &str,
) -> Result<(), AuthError> {
    conn.query_opt(
        "SELECT 1 FROM accounts WHERE id = $1::text::uuid FOR UPDATE",
        &[&account_id],
    )
    .await
    .map_err(map_query_error)?
    .ok_or(AuthError::InvalidSession)?;
    let is_admin = conn
        .query_opt(
            "SELECT is_admin FROM users
             WHERE id = $1::text::uuid AND account_id = $2::text::uuid",
            &[&actor_user_id, &account_id],
        )
        .await
        .map_err(map_query_error)?
        .ok_or(AuthError::InvalidSession)?
        .get::<_, bool>(0);
    if is_admin {
        Ok(())
    } else {
        Err(AuthError::Forbidden(ADMIN_ONLY))
    }
}

/// Sets `user_id`'s role and projects, after checking every project named is
/// on `account_id`. Call under [`lock_account_as_admin`], which is also what
/// keeps a project from being deleted between the check and the insert.
pub(crate) async fn write_user_access(
    conn: &Connection,
    account_id: &str,
    user_id: &str,
    access: &UserAccess,
) -> Result<(), AuthError> {
    if let ProjectScope::Only(ids) = &access.projects {
        let owned: i64 = conn
            .query_one(
                "SELECT COUNT(*) FROM projects
                 WHERE account_id = $1::text::uuid AND id = ANY($2)",
                &[&account_id, ids],
            )
            .await
            .map_err(map_query_error)?
            .get(0);
        if usize::try_from(owned).ok() != Some(ids.len()) {
            return Err(AuthError::InvalidRequest(
                "one of those projects isn't on this account",
            ));
        }
    }
    let all_projects = access.projects == ProjectScope::All;
    let updated = conn
        .execute(
            "UPDATE users SET is_admin = $3, all_projects = $4
             WHERE id = $1::text::uuid AND account_id = $2::text::uuid",
            &[&user_id, &account_id, &access.is_admin, &all_projects],
        )
        .await
        .map_err(map_query_error)?;
    if updated == 0 {
        return Err(AuthError::NotFound("no such user on this account"));
    }
    conn.execute(
        "DELETE FROM user_projects WHERE user_id = $1::text::uuid",
        &[&user_id],
    )
    .await
    .map_err(map_query_error)?;
    if let ProjectScope::Only(ids) = &access.projects {
        conn.execute(
            "INSERT INTO user_projects (user_id, project_id)
             SELECT $1::text::uuid, UNNEST($2::int4[])",
            &[&user_id, ids],
        )
        .await
        .map_err(map_query_error)?;
    }
    Ok(())
}

/// Confirms in Postgres that `user_id` is on `account_id` and may read
/// `project_key`, or is an admin when it is `None`, for minting a token
/// that outlives the session cache. Call inside
/// [`Connection::in_transaction`] and insert the token in the same one. The
/// share lock on the user's row makes an access change that is in flight wait
/// for the mint to commit, so its [`revoke_user_tokens`] then deletes the new
/// token, or makes the mint wait and then read the changed access.
///
/// The account row is locked first, as [`lock_account_as_admin`] does before
/// a change writes the user's row. The token insert's foreign key on the
/// account needs that lock anyway, and taking it after the user's row would
/// deadlock against a change holding the account and waiting on the user.
pub(crate) async fn lock_user_project_access(
    conn: &Connection,
    user_id: &str,
    account_id: &str,
    project_key: Option<i32>,
) -> Result<(), AuthError> {
    conn.query_opt(
        "SELECT 1 FROM accounts WHERE id = $1::text::uuid FOR KEY SHARE",
        &[&account_id],
    )
    .await
    .map_err(map_query_error)?
    .ok_or(AuthError::InvalidCredentials)?;
    check_user_project_access(conn, user_id, account_id, project_key, true).await
}

/// [`lock_user_project_access`] without the lock, for checking a grant that
/// is already held rather than minting one.
pub(crate) async fn user_project_access(
    conn: &Connection,
    user_id: &str,
    account_id: &str,
    project_key: Option<i32>,
) -> Result<(), AuthError> {
    check_user_project_access(conn, user_id, account_id, project_key, false).await
}

async fn check_user_project_access(
    conn: &Connection,
    user_id: &str,
    account_id: &str,
    project_key: Option<i32>,
    lock: bool,
) -> Result<(), AuthError> {
    let select = if lock {
        "SELECT is_admin, all_projects FROM users
         WHERE id = $1::text::uuid AND account_id = $2::text::uuid
         FOR SHARE"
    } else {
        "SELECT is_admin, all_projects FROM users
         WHERE id = $1::text::uuid AND account_id = $2::text::uuid"
    };
    let row = conn
        .query_opt(select, &[&user_id, &account_id])
        .await
        .map_err(map_query_error)?
        .ok_or(AuthError::InvalidCredentials)?;
    let (is_admin, all_projects): (bool, bool) = (row.get(0), row.get(1));
    let Some(project_key) = project_key else {
        return if is_admin {
            Ok(())
        } else {
            Err(AuthError::InvalidCredentials)
        };
    };
    if all_projects {
        return Ok(());
    }
    // A separate statement, so under READ COMMITTED it sees whatever change
    // the lock above waited for.
    let listed: bool = conn
        .query_one(
            "SELECT EXISTS (
                SELECT 1 FROM user_projects
                WHERE user_id = $1::text::uuid AND project_id = $2
             )",
            &[&user_id, &project_key],
        )
        .await
        .map_err(map_query_error)?
        .get(0);
    if listed {
        Ok(())
    } else {
        Err(AuthError::InvalidCredentials)
    }
}

/// Deletes `user_id`'s API tokens that the new access no longer covers. Query
/// tokens name one project, so they all go rather than checking each; the
/// dashboard mints a new one when it needs it. Only admins may connect an
/// MCP agent, so its tokens survive only while the user is still one.
pub(crate) async fn revoke_user_tokens(
    conn: &Connection,
    user_id: &str,
    access: &UserAccess,
) -> Result<(), AuthError> {
    let keep_mcp = access.is_admin;
    conn.execute(
        "DELETE FROM api_tokens
         WHERE user_id = $1::text::uuid AND (kind <> 'mcp' OR NOT $2)",
        &[&user_id, &keep_mcp],
    )
    .await
    .map_err(map_query_error)?;
    Ok(())
}

/// Stops this process serving what it cached about `user_id` after their
/// access changed or they were removed. Token caches hold only what a token
/// grants, not whose it is, so they are emptied whole and refill on the next
/// request. Call after the write whether or not it reported success, since an
/// error can follow a commit.
pub(crate) fn forget_user(server_context: &Arc<ServerContext>, user_id: &str) {
    server_context.session_cache.invalidate_user(user_id);
    server_context.query_token_cache.retain(|_| false);
    server_context.mcp_token_cache.retain(|_| false);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{insert_test_account, main_test_db};

    #[tokio::test]
    async fn a_mint_waits_for_an_access_change_instead_of_deadlocking() {
        let (server_context, admin_conn) = main_test_db("mint-lock").await;
        let account_id = insert_test_account(&admin_conn, "mint_lock").await;
        let project_name = format!("p{}", account_id.replace('-', ""));
        let project_key: i32 = admin_conn
            .query_one(
                "INSERT INTO projects (account_id, name) VALUES ($1::text::uuid, $2) RETURNING id",
                &[&account_id, &project_name],
            )
            .await
            .expect("insert project")
            .get(0);
        let user_id: String = admin_conn
            .query_one(
                "INSERT INTO users (account_id, name, email, is_admin)
                 VALUES ($1::text::uuid, 'Member', $2, FALSE) RETURNING id::text",
                &[&account_id, &format!("{project_name}@example.com")],
            )
            .await
            .expect("insert member")
            .get(0);
        let mint_conn = crate::db::open_main_database_write(server_context.clone())
            .await
            .expect("second connection");

        // The admin's change holds the account row, as lock_account_as_admin
        // does, while a mint for the member starts, then narrows the member.
        let (mint_user, mint_account) = (user_id.clone(), account_id.clone());
        let mint = admin_conn
            .in_transaction(async |conn| {
                conn.query_one(
                    "SELECT 1 FROM accounts WHERE id = $1::text::uuid FOR UPDATE",
                    &[&account_id],
                )
                .await?;
                let mint = tokio::spawn(async move {
                    mint_conn
                        .in_transaction(async |conn| {
                            lock_user_project_access(
                                conn,
                                &mint_user,
                                &mint_account,
                                Some(project_key),
                            )
                            .await?;
                            conn.execute(
                                "INSERT INTO api_tokens
                                    (token_hash, kind, user_id, account_id, project_id, expires_at)
                                 VALUES ('\\x01', 'query', $1::text::uuid, $2::text::uuid, $3,
                                    NOW() + INTERVAL '1 minute')",
                                &[&mint_user, &mint_account, &project_key],
                            )
                            .await?;
                            Ok::<_, AuthError>(())
                        })
                        .await
                });
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                conn.execute(
                    "UPDATE users SET all_projects = FALSE WHERE id = $1::text::uuid",
                    &[&user_id],
                )
                .await?;
                Ok::<_, AuthError>(mint)
            })
            .await
            .expect("the access change commits");
        let minted = mint.await.expect("mint task");
        assert!(
            matches!(minted, Err(AuthError::InvalidCredentials)),
            "the mint should read the narrowed access, got {minted:?}"
        );

        admin_conn
            .execute(
                "DELETE FROM accounts WHERE id = $1::text::uuid",
                &[&account_id],
            )
            .await
            .expect("delete account");
    }

    fn request(is_admin: bool, project_ids: Option<Vec<i32>>) -> AccessRequest {
        AccessRequest {
            is_admin,
            project_ids,
        }
    }

    #[test]
    fn both_fields_of_an_access_request_are_required() {
        let parse = |body: &str| serde_json::from_str::<AccessRequest>(body);
        assert!(parse(r#"{"is_admin": false}"#).is_err());
        assert!(parse(r#"{"project_ids": null}"#).is_err());
        let all = parse(r#"{"is_admin": false, "project_ids": null}"#).expect("explicit null");
        assert_eq!(all.project_ids, None);
        let some = parse(r#"{"is_admin": false, "project_ids": [4]}"#).expect("list");
        assert_eq!(some.project_ids, Some(vec![4]));
    }

    #[test]
    fn admins_always_reach_every_project() {
        let access = UserAccess::from_request(request(true, Some(vec![3]))).expect("access");
        assert_eq!(access.projects, ProjectScope::All);
        assert!(access.is_admin);
    }

    #[test]
    fn a_member_project_list_is_sorted_and_deduplicated() {
        let access = UserAccess::from_request(request(false, Some(vec![9, 2, 9]))).expect("ok");
        assert_eq!(access.projects, ProjectScope::Only(vec![2, 9]));
        assert!(!access.is_admin);
    }

    #[test]
    fn a_member_without_a_list_reaches_every_project() {
        let access = UserAccess::from_request(request(false, None)).expect("access");
        assert_eq!(access.projects, ProjectScope::All);
    }

    #[test]
    fn an_empty_project_list_is_refused_rather_than_read_as_all() {
        assert!(matches!(
            UserAccess::from_request(request(false, Some(vec![]))),
            Err(AuthError::InvalidRequest(_))
        ));
    }

    #[test]
    fn scope_allows_only_listed_projects() {
        let scope = ProjectScope::from_ids(Some(vec![7, 3]));
        assert!(scope.allows(3));
        assert!(scope.allows(7));
        assert!(!scope.allows(5));
        assert!(ProjectScope::from_ids(None).allows(5));
        assert!(!ProjectScope::from_ids(Some(vec![])).allows(5));
    }
}
