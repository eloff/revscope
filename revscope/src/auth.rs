use revscope_api::auth::API_TOKEN_PREFIX;
pub(crate) use revscope_api::auth::{
    ForgotPasswordRequest, LoginRequest, NewAccountRequest, SignupRequest,
};

use log::error;
use regex::Regex;
use serde::Deserialize;
use std::num::NonZeroU32;
use std::sync::{Arc, LazyLock};
use tokio_postgres::error::SqlState;

use crate::account_subscription::AccountSubscriptionError;
use crate::db::{DbError, PostgresError, open_main_database_read, open_main_database_write};
use crate::server_context::ServerContext;
use crate::tokens::{
    API_TOKEN_TTL_SECONDS, PASSWORD_RESET_TTL_SECONDS, SESSION_MAX_AGE_SECONDS, TokenSecret,
};
use crate::user_access::ProjectScope;
use crate::uuid_token::UuidToken;

/// OWASP's figure for PBKDF2-HMAC-SHA256. Each stored hash records its own
/// count, so hashes made with an older count still verify and are rehashed at
/// this one on the user's next login.
const PASSWORD_HASHING_ROUNDS: NonZeroU32 = NonZeroU32::new(600_000).unwrap();
const PASSWORD_HASH_LEN: usize = 32;
const PASSWORD_HASH_RECORD_LEN: usize = 4 + PASSWORD_HASH_LEN;
pub(crate) const MIN_USER_NAME_LENGTH: usize = 2;
const MIN_PASSWORD_LENGTH: usize = 10;

/// The user a signup creates, already validated.
pub(crate) struct NewUser<'a> {
    pub(crate) name: &'a str,
    pub(crate) email: NormalizedEmail,
    pub(crate) login: NewLogin<'a>,
}

pub(crate) enum NewLogin<'a> {
    Password(&'a str),
    /// Signed up with Google or Apple; no password until they set one from a reset link.
    Identity {
        provider: &'static crate::id_token::Provider,
        subject: &'a str,
    },
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResetPasswordRequest {
    pub(crate) token: String,
    pub(crate) password: String,
}

#[derive(Debug)]
pub(crate) struct AuthSuccess {
    pub(crate) session: SessionCookie,
}

#[derive(Debug, Clone)]
pub(crate) struct SessionCookie {
    pub(crate) session_id: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ProjectSettings {
    pub(crate) timezone: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ProjectInfo {
    pub(crate) id: String,
    pub(crate) settings: ProjectSettings,
}

/// The signed-in user behind a session: what authorization needs on every
/// request. One query, no project list, and usually not even that: see
/// [`crate::session_cache`].
#[derive(Debug, Clone)]
pub(crate) struct SessionUser {
    pub(crate) user_id: String,
    pub(crate) account_id: String,
    pub(crate) name: String,
    pub(crate) email: String,
    pub(crate) plan: String,
    pub(crate) is_admin: bool,
    pub(crate) projects: ProjectScope,
}

/// The full `/v2/session/me` payload: the session user plus the account's
/// projects and avatar.
#[derive(Debug)]
pub(crate) struct MeSuccess {
    pub(crate) user: SessionUser,
    /// Every project on the account the user may read, as `(name, info)`,
    /// ordered by name.
    pub(crate) projects: Vec<(String, ProjectInfo)>,
    pub(crate) gravatar_url: String,
}

/// A signup the user can fix in the form and that wrote nothing: a field the
/// server rejected, or a project name that's taken. Neither uses up one of the
/// address's signups.
pub(crate) fn is_fixable_signup_error(error: &AuthError) -> bool {
    match error {
        AuthError::InvalidRequest(_) => true,
        AuthError::Conflict(message) => *message == crate::projects::PROJECT_NAME_TAKEN,
        _ => false,
    }
}

#[derive(Debug)]
pub(crate) enum AuthError {
    InvalidRequest(&'static str),
    InvalidCredentials,
    Conflict(&'static str),
    Forbidden(&'static str),
    /// The caller may use the route, but has nothing by that id. Also what a
    /// record owned by someone else reports, so its existence is not revealed.
    NotFound(&'static str),
    Temporary(&'static str),
    Db(String),
    Query(String),
    InvalidHashFormat,
    InvalidSession,
}

impl From<DbError> for AuthError {
    fn from(value: DbError) -> Self {
        Self::Db(value.to_string())
    }
}

impl From<PostgresError> for AuthError {
    fn from(error: PostgresError) -> Self {
        Self::Query(error.to_string())
    }
}

impl From<tokio_postgres::Error> for AuthError {
    fn from(error: tokio_postgres::Error) -> Self {
        PostgresError::from(error).into()
    }
}

pub(crate) async fn signup(
    server_context: Arc<ServerContext>,
    payload: SignupRequest,
) -> Result<AuthSuccess, AuthError> {
    let name = payload.name.trim();
    if name.len() < MIN_USER_NAME_LENGTH {
        return Err(AuthError::InvalidRequest(
            "name must be at least 2 characters",
        ));
    }
    let password = payload.password.as_str();
    #[cfg(not(feature = "self-hosted"))]
    crate::hosted_signup::validate_account(&payload.account)?;
    // A signup through the CLI may leave the password out, since it holds a token.
    // It gets one nobody knows rather than none: a user with no password and no
    // sign-in is a pending invite, which any admin may send a new reset link to.
    // Its owner claims it with their own reset link. Self-hosted stays password-only:
    // a reset link needs outgoing email, which an instance may not have, and this is
    // its only admin.
    let unknowable;
    let password =
        if password.is_empty() && payload.issue_token && cfg!(not(feature = "self-hosted")) {
            unknowable = TokenSecret::generate()?.to_string();
            unknowable.as_str()
        } else {
            validate_new_password(password)?;
            password
        };
    let user = NewUser {
        name,
        email: normalize_email(payload.email.as_str())?,
        login: NewLogin::Password(password),
    };
    create_account(server_context, &payload.account, user).await
}

/// Creates the account a signup asked for, with `user` as its first user, and
/// signs them in.
pub(crate) async fn create_account(
    server_context: Arc<ServerContext>,
    account: &NewAccountRequest,
    user: NewUser<'_>,
) -> Result<AuthSuccess, AuthError> {
    #[cfg(feature = "self-hosted")]
    return self_hosted_signup(server_context, account, user).await;
    #[cfg(not(feature = "self-hosted"))]
    return crate::hosted_signup::signup(server_context, account, user).await;
}

/// Signup on a self-hosted build: no plan, and the user joins the instance's
/// one shared account, whose first project is named as the signup chose. Only
/// the instance's first user may sign up; the rest are invited. Compiled into
/// every build, like the rest of `self_hosted.rs`, but only called by a
/// self-hosted one.
#[cfg_attr(not(feature = "self-hosted"), allow(dead_code))]
async fn self_hosted_signup(
    server_context: Arc<ServerContext>,
    account: &NewAccountRequest,
    user: NewUser<'_>,
) -> Result<AuthSuccess, AuthError> {
    let timezone = timezone_for_signup(account.timezone.as_deref());
    let conn = open_main_database_write(server_context.clone()).await?;
    if !crate::self_hosted::signup_is_open(&conn).await? {
        return Err(AuthError::Forbidden(crate::self_hosted::SIGNUP_CLOSED));
    }
    let project_name = crate::projects::signup_project_name(
        &account.account_name,
        account.project_name.as_deref(),
    )?;

    crate::self_hosted::lock_signup(&conn).await?;
    let result = async {
        // Checked again under the lock: a racing first signup may have won.
        if !crate::self_hosted::signup_is_open(&conn).await? {
            return Err(AuthError::Forbidden(crate::self_hosted::SIGNUP_CLOSED));
        }
        conn.in_transaction(async |conn| {
            let (account_id, project_id) =
                crate::self_hosted::signup_account(conn, &project_name, &timezone).await?;
            let user_id = insert_new_user(conn, &account_id, &user).await?;
            Ok::<_, AuthError>((account_id, project_id, user_id))
        })
        .await
    }
    .await;
    crate::self_hosted::unlock_signup(&conn).await;
    let (account_id, project_id, user_id) = result?;
    drop(conn);
    let account_key = crate::db::project::parse_uuid_key(&account_id)
        .expect("postgres account UUID text must be a valid UUID");
    crate::db::insert_project(
        server_context.as_ref(),
        project_id,
        project_name,
        account_key,
    );
    issue_session(server_context, &user_id).await
}

/// Inserts `user` into `account_id` with their login, returning their id.
/// Only signups call this, so the user is the new account's admin.
pub(crate) async fn insert_new_user(
    conn: &crate::db::main::Connection,
    account_id: &str,
    user: &NewUser<'_>,
) -> Result<String, AuthError> {
    match user.login {
        NewLogin::Password(password) => {
            insert_user(
                conn,
                account_id,
                user.name,
                &user.email,
                Some(password),
                true,
            )
            .await
        }
        NewLogin::Identity { provider, subject } => {
            let user_id = insert_user(conn, account_id, user.name, &user.email, None, true).await?;
            let linked = link_identity(conn, provider.id, subject, &user_id).await;
            if !matches!(linked, Ok(true)) {
                // Not one transaction: leave no user behind that nothing can log in as.
                let _ = conn
                    .execute("DELETE FROM users WHERE id = $1::text::uuid", &[&user_id])
                    .await;
                // A racing signup with the same sign-in got there first.
                linked?;
                return Err(AuthError::Conflict(provider.already_linked));
            }
            Ok(user_id)
        }
    }
}

/// Lets `subject` at `provider` sign in as `user_id`. A subject already linked
/// (to anyone) is left as it was, and answers false.
async fn link_identity(
    conn: &crate::db::main::Connection,
    provider: &str,
    subject: &str,
    user_id: &str,
) -> Result<bool, AuthError> {
    let inserted = conn
        .execute(
            "INSERT INTO user_identities (provider, subject, user_id)
         VALUES ($1, $2, $3::text::uuid)
         ON CONFLICT (provider, subject) DO NOTHING",
            &[&provider, &subject, &user_id],
        )
        .await
        .map_err(map_query_error)?;
    Ok(inserted == 1)
}

/// Inserts a user into `account_id` and returns their id. Without a password
/// they log in with Google or Apple (see [`insert_new_user`]), or, as invites
/// work, not at all until they set one from a reset link. Whoever signs up is
/// the account's admin; an invite chooses.
pub(crate) async fn insert_user(
    conn: &crate::db::main::Connection,
    account_id: &str,
    name: &str,
    email_parts: &NormalizedEmail,
    password: Option<&str>,
    is_admin: bool,
) -> Result<String, AuthError> {
    let user_id_row = conn
        .query_one("SELECT uuidv7()::text", &[])
        .await
        .map_err(map_query_error)?;
    let user_id: String = user_id_row.get(0);
    let hash = password.map(|password| {
        hash_password(
            password,
            &email_parts.salt_email,
            &user_id,
            account_id,
            PASSWORD_HASHING_ROUNDS,
        )
    });

    let insert_user_result = conn
        .execute(
            "INSERT INTO users (id, account_id, name, email, original_email, hashed_password, is_admin)
             VALUES ($1::text::uuid, $2::text::uuid, $3, $4, $5, $6, $7)",
            &[
                &user_id,
                &account_id,
                &name,
                &email_parts.normalized,
                &email_parts.original,
                &hash,
                &is_admin,
            ],
        )
        .await;
    match insert_user_result {
        Ok(_) => Ok(user_id),
        Err(error) if is_timestamp_conflict(&error, "users_pkey") => Err(AuthError::Temporary(
            "a temporary error occurred, please try again",
        )),
        Err(error)
            if error.code() == Some(&SqlState::UNIQUE_VIOLATION)
                && db_constraint(&error) == Some("users_email_idx") =>
        {
            Err(AuthError::Conflict(
                "An account with this email already exists. Log in instead.",
            ))
        }
        Err(error) => Err(map_query_error(error)),
    }
}

pub(crate) async fn login(
    server_context: Arc<ServerContext>,
    payload: LoginRequest,
) -> Result<AuthSuccess, AuthError> {
    let username = payload.username.trim().to_ascii_lowercase();
    // Signup and reset used to trim before hashing and to allow whitespace, so
    // accounts from then still need the trim here and no whitespace rejection.
    let password = payload.password.trim();
    if username.is_empty() || password.is_empty() {
        return Err(AuthError::InvalidRequest(
            "username and password are required",
        ));
    }

    let conn = open_main_database_read(server_context.clone()).await?;
    let row = conn
        .query_opt(
            "SELECT
                id::text,
                account_id::text,
                COALESCE(original_email, email),
                hashed_password
             FROM users
             WHERE email = $1
             LIMIT 1",
            &[&username],
        )
        .await
        .map_err(map_query_error)?;
    drop(conn);

    // Without a hash to check, one is computed anyway so an unknown email (or
    // an invited user with no password yet) takes as long as a wrong password,
    // and response times don't reveal which emails have accounts.
    let Some((row, hashed_password)) =
        row.and_then(|row| row.get::<_, Option<Vec<u8>>>(3).map(|hash| (row, hash)))
    else {
        std::hint::black_box(hash_password(password, "", "", "", PASSWORD_HASHING_ROUNDS));
        return Err(AuthError::InvalidCredentials);
    };
    let user_id: String = row.get(0);
    let account_id: String = row.get(1);
    let salt_email: String = row.get(2);

    let rounds = stored_hash_rounds(&hashed_password)?;
    let matches = verify_password(
        password,
        hashed_password.as_slice(),
        &salt_email,
        &user_id,
        &account_id,
    )?;
    if !matches {
        // Tops a check against an older, cheaper hash up to the current round
        // count, so a wrong password costs what an unknown email does.
        if let Some(missing) = PASSWORD_HASHING_ROUNDS
            .get()
            .checked_sub(rounds.get())
            .and_then(NonZeroU32::new)
        {
            std::hint::black_box(hash_password(password, "", "", "", missing));
        }
        return Err(AuthError::InvalidCredentials);
    }
    if rounds < PASSWORD_HASHING_ROUNDS {
        let rehashed = hash_password(
            password,
            &salt_email,
            &user_id,
            &account_id,
            PASSWORD_HASHING_ROUNDS,
        );
        // A failed upgrade leaves the old hash working, so it must not fail
        // the login; the next one tries again.
        if let Err(error) = replace_password_hash(
            server_context.clone(),
            &user_id,
            &hashed_password,
            &rehashed,
        )
        .await
        {
            error!("failed to upgrade password hash for user {user_id}: {error:?}");
        }
    }

    issue_session(server_context, &user_id).await
}

/// Swaps `old` for `new` only if it is still the stored hash, so a password
/// reset that lands between the read and this write is not undone.
async fn replace_password_hash(
    server_context: Arc<ServerContext>,
    user_id: &str,
    old: &[u8],
    new: &[u8],
) -> Result<(), AuthError> {
    let conn = open_main_database_write(server_context).await?;
    conn.execute(
        "UPDATE users SET hashed_password = $3
         WHERE id = $1::text::uuid AND hashed_password = $2",
        &[&user_id, &old, &new],
    )
    .await
    .map_err(map_query_error)?;
    Ok(())
}

/// Turns a session just issued into a long-lived API token. The same row, so it
/// is revoked the way any session is and authorizes the same routes.
pub(crate) async fn extend_session_to_api_token(
    server_context: Arc<ServerContext>,
    session: &SessionCookie,
) -> Result<String, AuthError> {
    let token = session_token(session)?;
    let conn = open_main_database_write(server_context).await?;
    conn.execute(
        "UPDATE sessions
         SET expires_at = NOW() + ($2::int * INTERVAL '1 second')
         WHERE token_hash = $1",
        &[&token.hash(), &API_TOKEN_TTL_SECONDS],
    )
    .await
    .map_err(map_query_error)?;
    Ok(format!("{API_TOKEN_PREFIX}{}", session.session_id))
}

/// Create a session shared by signup and login.
pub(crate) async fn issue_session(
    server_context: Arc<ServerContext>,
    user_id: &str,
) -> Result<AuthSuccess, AuthError> {
    let conn = open_main_database_write(server_context.clone()).await?;
    conn.execute(
        "UPDATE users SET last_login_at = NOW() WHERE id = $1::text::uuid",
        &[&user_id],
    )
    .await
    .map_err(map_query_error)?;
    let token = UuidToken::generate()?;
    conn.execute(
        "INSERT INTO sessions (token_hash, user_id, expires_at)
         VALUES ($1, $2::text::uuid, NOW() + ($3::int * INTERVAL '1 second'))",
        &[&token.hash(), &user_id, &SESSION_MAX_AGE_SECONDS],
    )
    .await
    .map_err(map_query_error)?;
    drop(conn);

    Ok(AuthSuccess {
        session: SessionCookie {
            session_id: token.to_string(),
        },
    })
}

/// Deletes the session so the cookie the client is about to drop cannot be
/// replayed. Unknown or malformed sessions are a no-op: the caller clears the
/// cookie either way.
pub(crate) async fn logout(
    server_context: Arc<ServerContext>,
    session: &SessionCookie,
) -> Result<(), AuthError> {
    let Some(token) = UuidToken::parse(session.session_id.as_str()) else {
        return Ok(());
    };
    let session_key = token.key();
    let conn = open_main_database_write(server_context.clone()).await?;
    let deleted = conn
        .execute(
            "DELETE FROM sessions WHERE token_hash = $1",
            &[&token.hash()],
        )
        .await;
    // After the delete, so a concurrent lookup cannot cache the row again.
    // Only a delete that may have removed a row bumps the generation (an error
    // can follow a commit): this route is public, and a made-up cookie bumping
    // it would stop every concurrent lookup from caching.
    match deleted {
        Ok(0) => server_context.session_cache.evict(session_key),
        Ok(_) | Err(_) => server_context.session_cache.remove(session_key),
    }
    deleted.map_err(map_query_error)?;
    Ok(())
}

/// Answers as soon as the email is validated, and looks the user up and sends
/// the email in the background. Doing that work before answering would make
/// the response measurably slower for emails that have an account.
pub(crate) fn request_password_reset(
    server_context: Arc<ServerContext>,
    payload: ForgotPasswordRequest,
) -> Result<(), AuthError> {
    let email_parts = normalize_email(payload.email.as_str())?;
    // Dropped rather than queued when full: the caller has already been
    // answered, so a flood would otherwise pile up work nothing pushes back on.
    let Ok(permit) = server_context
        .password_reset_sends
        .clone()
        .try_acquire_owned()
    else {
        log::warn!("password reset dropped: too many already in progress");
        return Ok(());
    };
    crate::task::spawn(server_context.clone(), async move {
        let _permit = permit;
        if let Err(error) = send_password_reset(server_context, &email_parts.normalized).await {
            crate::sentry_report::report_error!(
                "auth.forgot_password",
                "failed to start a password reset: {error:?}"
            );
        }
    });
    Ok(())
}

async fn send_password_reset(
    server_context: Arc<ServerContext>,
    normalized_email: &str,
) -> Result<(), AuthError> {
    let conn = open_main_database_read(server_context.clone()).await?;
    let row = conn
        .query_opt(
            "SELECT id::text, name, COALESCE(original_email, email)
             FROM users
             WHERE email = $1
             LIMIT 1",
            &[&normalized_email],
        )
        .await
        .map_err(map_query_error)?;
    drop(conn);

    let Some(row) = row else {
        return Ok(());
    };
    let user_id: String = row.get(0);
    let name: String = row.get(1);
    let email: String = row.get(2);

    let conn = open_main_database_write(server_context.clone()).await?;
    conn.execute(
        "DELETE FROM password_reset_tokens
         WHERE user_id = $1::text::uuid
         AND expires_at < NOW()",
        &[&user_id],
    )
    .await
    .map_err(map_query_error)?;
    let token = insert_password_reset_token(&conn, &user_id, PASSWORD_RESET_TTL_SECONDS).await?;
    drop(conn);

    let reset_url = password_reset_url(&token);
    if let Err(send_error) = crate::email::send_password_reset_email(
        &server_context.http,
        &email,
        &name,
        reset_url.as_str(),
    )
    .await
    {
        error!("failed to send password reset email: {send_error}");
        delete_password_reset_token(server_context, &token).await;
    }

    Ok(())
}

pub(crate) async fn reset_password(
    server_context: Arc<ServerContext>,
    payload: ResetPasswordRequest,
) -> Result<(), AuthError> {
    let token = payload.token.trim();
    if token.is_empty() {
        return Err(AuthError::InvalidRequest("reset token is required"));
    }
    let Some(token) = UuidToken::parse(token) else {
        return Err(AuthError::InvalidRequest(
            "password reset link is invalid or expired",
        ));
    };
    let token_hash = token.hash();
    let password = payload.password.as_str();
    validate_new_password(password)?;

    let conn = open_main_database_read(server_context.clone()).await?;
    let row = conn
        .query_opt(
            "SELECT u.id::text, u.account_id::text, COALESCE(u.original_email, u.email)
             FROM password_reset_tokens t
             INNER JOIN users u ON u.id = t.user_id
             WHERE t.token_hash = $1
             AND t.expires_at > NOW()
             LIMIT 1",
            &[&token_hash],
        )
        .await
        .map_err(map_query_error)?
        .ok_or(AuthError::InvalidRequest(
            "password reset link is invalid or expired",
        ))?;
    drop(conn);

    let user_id: String = row.get(0);
    let account_id: String = row.get(1);
    let salt_email: String = row.get(2);
    let password_hash = hash_password(
        password,
        &salt_email,
        &user_id,
        &account_id,
        PASSWORD_HASHING_ROUNDS,
    );

    let conn = open_main_database_write(server_context.clone()).await?;
    let updated = conn
        .in_transaction(async |conn| {
            // Locked first, as a re-invite does, so a token the re-invite
            // replaces while this waits is read as gone by the statement below
            // (a new snapshot) instead of setting the password anyway.
            conn.execute(
                "SELECT 1 FROM users WHERE id = $1::text::uuid FOR UPDATE",
                &[&user_id],
            )
            .await
            .map_err(map_query_error)?;
            let reset = conn
                .query_opt(
                    "WITH valid_token AS (
                        SELECT token_hash, user_id
                        FROM password_reset_tokens
                        WHERE token_hash = $1
                        AND expires_at > NOW()
                        LIMIT 1
                     ),
                     updated_user AS (
                        UPDATE users
                        SET hashed_password = $2
                        WHERE id = (SELECT user_id FROM valid_token)
                        RETURNING id
                     ),
                     deleted_token AS (
                        DELETE FROM password_reset_tokens
                        WHERE token_hash = (SELECT token_hash FROM valid_token)
                        AND EXISTS (SELECT 1 FROM updated_user)
                        RETURNING user_id
                     ),
                     deleted_sessions AS (
                        DELETE FROM sessions
                        WHERE user_id = (SELECT user_id FROM deleted_token)
                        RETURNING user_id
                     )
                     SELECT user_id::text FROM deleted_token",
                    &[&token_hash, &password_hash],
                )
                .await
                .map_err(map_query_error)?;
            // Rolled back unless the token was spent, so a password is never
            // set by a link the caller is told didn't work.
            reset.map(|_| ()).ok_or(AuthError::InvalidRequest(
                "password reset link is invalid or expired",
            ))
        })
        .await;
    // The reset deletes every session of this user, so stop serving them here.
    // Before looking at the result, since an error can follow a commit. The
    // token can only name the user read above.
    server_context
        .session_cache
        .invalidate_user(user_id.as_str());
    updated?;

    Ok(())
}

/// Resolves a session to its user. This is what authorization calls; a
/// project's owner comes from the `ProjectCache`, and the user's role and
/// project scope from the row loaded here.
///
/// A session Postgres confirmed within
/// [`crate::session_cache::SESSION_CACHE_TTL_SECONDS`] is answered from memory
/// without taking a connection.
pub(crate) async fn session_user(
    server_context: Arc<ServerContext>,
    session: &SessionCookie,
) -> Result<SessionUser, AuthError> {
    let token = session_token(session)?;
    if let Some(user) = server_context.session_cache.get(token.key()) {
        return Ok(user);
    }
    let conn = open_main_database_read(server_context.clone()).await?;
    load_session_user(&server_context, &conn, token).await
}

/// The Postgres half of [`session_user`]: queries on `conn` and caches what it
/// finds. A session Postgres does not know is never cached.
async fn load_session_user(
    server_context: &ServerContext,
    conn: &crate::db::main::Connection,
    token: UuidToken,
) -> Result<SessionUser, AuthError> {
    let generation = server_context.session_cache.generation();
    let row = conn
        .query_opt(
            "SELECT
                u.id::text,
                u.account_id::text,
                u.name,
                COALESCE(u.original_email, u.email) AS email,
                sub.plan_type::text,
                -- trunc: ::bigint alone rounds, so a cache entry could
                -- outlive the session by up to half a second.
                trunc(EXTRACT(EPOCH FROM s.expires_at))::bigint,
                u.is_admin,
                CASE WHEN u.all_projects THEN NULL ELSE ARRAY(
                    SELECT up.project_id FROM user_projects up WHERE up.user_id = u.id
                ) END
             FROM sessions s
             INNER JOIN users u ON u.id = s.user_id
             INNER JOIN accounts a ON a.id = u.account_id
             INNER JOIN account_subscriptions sub
                ON sub.account_id = a.id AND sub.ended_at IS NULL
             WHERE s.token_hash = $1
             AND s.expires_at > NOW()
             LIMIT 1",
            &[&token.hash()],
        )
        .await
        .map_err(map_query_error)?
        .ok_or(AuthError::InvalidSession)?;

    let user = SessionUser {
        user_id: row.get(0),
        account_id: row.get(1),
        name: row.get(2),
        email: row.get(3),
        plan: row.get(4),
        is_admin: row.get(6),
        projects: ProjectScope::from_ids(row.get(7)),
    };
    let session_expires_at = u32::try_from(row.get::<_, i64>(5)).unwrap_or(0);
    server_context
        .session_cache
        .insert(token.key(), user.clone(), session_expires_at, generation);
    Ok(user)
}

/// A verified session user plus the projects they may read, for
/// `/v2/session/me` and the signup and login answers.
pub(crate) async fn me(
    server_context: Arc<ServerContext>,
    user: SessionUser,
) -> Result<MeSuccess, AuthError> {
    let conn = open_main_database_read(server_context).await?;

    let project_rows = conn
        .query(
            "SELECT
                p.name,
                p.id::text,
                COALESCE(ps.timezone, 'UTC')
             FROM projects p
             LEFT JOIN project_settings ps ON ps.project_id = p.id
             WHERE p.account_id = $1::text::uuid
             AND ($2::int4[] IS NULL OR p.id = ANY($2))
             ORDER BY p.name",
            &[&user.account_id, &user.projects.project_ids()],
        )
        .await
        .map_err(map_query_error)?;
    let projects = project_rows
        .iter()
        .map(|row| {
            (
                row.get::<_, String>(0),
                ProjectInfo {
                    id: row.get(1),
                    settings: ProjectSettings {
                        timezone: row.get(2),
                    },
                },
            )
        })
        .collect();

    Ok(MeSuccess {
        gravatar_url: gravatar_url(user.email.as_str()),
        user,
        projects,
    })
}

/// The token a session cookie carries, or `InvalidSession` when it is not one
/// we could have issued. Whether it is live is for Postgres to say.
pub(crate) fn session_token(session: &SessionCookie) -> Result<UuidToken, AuthError> {
    UuidToken::parse(session.session_id.as_str()).ok_or(AuthError::InvalidSession)
}

pub(crate) struct NormalizedEmail {
    pub(crate) normalized: String,
    pub(crate) original: Option<String>,
    salt_email: String,
}

pub(crate) fn normalize_email(raw_email: &str) -> Result<NormalizedEmail, AuthError> {
    let trimmed = raw_email.trim();
    if trimmed.is_empty() {
        return Err(AuthError::InvalidRequest("email is required"));
    }
    if !trimmed.contains('@') {
        return Err(AuthError::InvalidRequest("email must be valid"));
    }
    let normalized = trimmed.to_ascii_lowercase();
    let original = if normalized == trimmed {
        None
    } else {
        Some(trimmed.to_string())
    };
    let salt_email = original.clone().unwrap_or_else(|| normalized.clone());
    Ok(NormalizedEmail {
        normalized,
        original,
        salt_email,
    })
}

/// The same pattern as FORBIDDEN_PASSWORD_CHARACTER in authValidation.ts:
/// whitespace, control, format, private use, noncharacters and default-ignorable
/// code points. Nearly all are invisible or render differently between devices,
/// so a password holding one, as from a pasted or piped value, may not be
/// retyped the same way at login. Other unassigned code points are allowed
/// because this crate and browsers ship different Unicode versions, so `\p{Cn}`
/// would refuse new emoji. Format characters can also differ by version, but
/// new ones are rare and the only effect is that this check refuses one the
/// form let through.
static FORBIDDEN_PASSWORD_CHARACTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"[\p{Cc}\p{Cf}\p{Co}\p{Z}\p{Default_Ignorable_Code_Point}\p{Noncharacter_Code_Point}]",
    )
    .expect("the password character pattern compiles")
});

// Complexity / uniqueness rules are enforced on the client only so they do not
// drift out of sync with authValidation.ts.
pub(crate) fn validate_new_password(password: &str) -> Result<(), AuthError> {
    if password.len() < MIN_PASSWORD_LENGTH {
        return Err(AuthError::InvalidRequest(
            "password must be at least 10 characters",
        ));
    }
    if FORBIDDEN_PASSWORD_CHARACTER.is_match(password) {
        return Err(AuthError::InvalidRequest(
            "password cannot contain whitespace or non-printable characters",
        ));
    }
    Ok(())
}

fn hash_password(
    password: &str,
    salt_email: &str,
    user_id: &str,
    account_id: &str,
    rounds: NonZeroU32,
) -> Vec<u8> {
    let derived = derive_password_key(password, salt_email, user_id, account_id, rounds);
    let mut stored_hash = Vec::with_capacity(PASSWORD_HASH_RECORD_LEN);
    stored_hash.extend_from_slice(&rounds.get().to_be_bytes());
    stored_hash.extend_from_slice(&derived);
    stored_hash
}

fn verify_password(
    password: &str,
    stored_hash: &[u8],
    salt_email: &str,
    user_id: &str,
    account_id: &str,
) -> Result<bool, AuthError> {
    let rounds = stored_hash_rounds(stored_hash)?;
    let expected_derived = derive_password_key(password, salt_email, user_id, account_id, rounds);
    Ok(crate::crypto::constant_time_eq(
        &stored_hash[4..],
        expected_derived.as_slice(),
    ))
}

/// The round count a stored hash record was made with.
fn stored_hash_rounds(stored_hash: &[u8]) -> Result<NonZeroU32, AuthError> {
    if stored_hash.len() != PASSWORD_HASH_RECORD_LEN {
        return Err(AuthError::InvalidHashFormat);
    }
    let rounds = u32::from_be_bytes(
        stored_hash[..4]
            .try_into()
            .map_err(|_| AuthError::InvalidHashFormat)?,
    );
    NonZeroU32::new(rounds).ok_or(AuthError::InvalidHashFormat)
}

/// PBKDF2-HMAC-SHA256 from aws-lc rather than the RustCrypto crates: theirs is
/// generic, so it compiles unoptimized into this crate in dev builds and takes
/// seconds per login, which times out the e2e tests. aws-lc's is prebuilt C.
fn derive_password_key(
    password: &str,
    salt_email: &str,
    user_id: &str,
    account_id: &str,
    rounds: NonZeroU32,
) -> [u8; PASSWORD_HASH_LEN] {
    let salt = format!("{salt_email}:{user_id}:{account_id}");
    let mut derived = [0_u8; PASSWORD_HASH_LEN];
    aws_lc_rs::pbkdf2::derive(
        aws_lc_rs::pbkdf2::PBKDF2_HMAC_SHA256,
        rounds,
        salt.as_bytes(),
        password.as_bytes(),
        &mut derived,
    );
    derived
}

pub(crate) fn password_reset_url(token: &UuidToken) -> String {
    format!(
        "{}/reset-password?token={token}",
        crate::util::web_base_url()
    )
}

/// Stores a reset token for `user_id` that lasts `ttl_seconds`, and returns the
/// plaintext for the emailed link; only its hash is kept.
pub(crate) async fn insert_password_reset_token(
    conn: &crate::db::main::Connection,
    user_id: &str,
    ttl_seconds: i32,
) -> Result<UuidToken, AuthError> {
    let token = UuidToken::generate()?;
    conn.execute(
        // Without the cast Postgres infers a float8 parameter, because that is
        // the preferred operand type for multiplying an interval.
        "INSERT INTO password_reset_tokens (token_hash, user_id, expires_at)
         VALUES ($1, $2::text::uuid, NOW() + ($3::int * INTERVAL '1 second'))",
        &[&token.hash(), &user_id, &ttl_seconds],
    )
    .await
    .map_err(map_query_error)?;
    Ok(token)
}

async fn delete_password_reset_token(server_context: Arc<ServerContext>, token: &UuidToken) {
    let Ok(conn) = open_main_database_write(server_context).await else {
        return;
    };
    let _ = conn
        .execute(
            "DELETE FROM password_reset_tokens WHERE token_hash = $1",
            &[&token.hash()],
        )
        .await;
}

fn gravatar_url(email: &str) -> String {
    let normalized = email.trim().to_ascii_lowercase();
    let hash = format!("{:x}", md5::compute(normalized));
    format!("https://www.gravatar.com/avatar/{hash}?d=mp&s=80")
}

/// The browser-reported zone, or UTC when it is missing or not a zone we can
/// resolve. Signup must not fail over a detail the user never typed.
pub(crate) fn timezone_for_signup(raw: Option<&str>) -> String {
    raw.and_then(crate::timezone::validate)
        .map(|timezone| timezone.name().to_string())
        .unwrap_or_else(|| crate::timezone::DEFAULT_TIMEZONE.to_string())
}

/// For SQLite reads, where a missing row is as unexpected as any other failure.
pub(crate) fn map_row_error(error: rusqlite::Error) -> AuthError {
    if matches!(error, rusqlite::Error::QueryReturnedNoRows) {
        AuthError::Db("sqlite query returned no row".to_string())
    } else {
        AuthError::Db(error.to_string())
    }
}

pub(crate) fn map_query_error(error: tokio_postgres::Error) -> AuthError {
    error.into()
}

pub(crate) fn map_account_subscription_error(error: AccountSubscriptionError) -> AuthError {
    match error {
        AccountSubscriptionError::OverlappingPeriods => {
            AuthError::Db("account subscription periods overlap".to_string())
        }
        AccountSubscriptionError::Db(message) => AuthError::Db(message),
    }
}

pub(crate) fn is_timestamp_conflict(error: &tokio_postgres::Error, constraint: &str) -> bool {
    error.code() == Some(&SqlState::UNIQUE_VIOLATION) && db_constraint(error) == Some(constraint)
}

pub(crate) fn db_constraint(error: &tokio_postgres::Error) -> Option<&str> {
    error
        .as_db_error()
        .and_then(tokio_postgres::error::DbError::constraint)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same code points as the list in authValidation.test.ts.
    const FORBIDDEN_CODE_POINTS: [u32; 31] = [
        0x20, 0x9, 0xa, 0xd, 0x85, 0xa0, 0x2028, 0x3000, // whitespace
        0x0, 0x8, 0x1b, 0x7f, 0x9f, // control
        0xad, 0x200b, 0x200c, 0x200d, 0x200e, 0x202e, 0x2060, 0xfeff, 0xe0001,
        0xfff9, // format
        0x34f, 0xfe0f, 0x115f, 0x3164, // default ignorable
        0xe000, 0xf0000, // private use
        0xfdd0, 0x10ffff, // noncharacter
    ];

    #[test]
    fn new_passwords_reject_forbidden_characters() {
        for code_point in FORBIDDEN_CODE_POINTS {
            let character = char::from_u32(code_point).unwrap();
            let password = format!("long-enough{character}password");
            assert!(
                validate_new_password(&password).is_err(),
                "accepted U+{code_point:04X}"
            );
        }
    }

    #[test]
    fn new_passwords_accept_printable_and_unassigned_characters() {
        for password in [
            "correctHorse9!~`@#$%^&*()_+-=[]{}|;:'\",.<>/?\\",
            "contraseña-çafé-ß",
            "пароль-достаточной-длины",
            "長いパスワードです長いパスワード",
            "long-enough-password-😀",
            "long-enough-password-\u{378}",
        ] {
            assert!(validate_new_password(password).is_ok(), "{password}");
        }
    }

    /// Hashes already stored were made by the RustCrypto `pbkdf2` crate, so
    /// this pins the output to an independent PBKDF2-HMAC-SHA256 (Python's
    /// `hashlib.pbkdf2_hmac`) for the same inputs.
    #[test]
    fn password_hashes_match_standard_pbkdf2_and_record_their_rounds() {
        let rounds = NonZeroU32::new(100_000).unwrap();
        let hash = hash_password(
            "correct horse battery",
            "Ada@Example.com",
            "user-id",
            "account-id",
            rounds,
        );
        assert_eq!(&hash[..4], &100_000_u32.to_be_bytes());
        assert_eq!(
            crate::util::encode_lower_hex(&hash[4..]),
            "b2d9a4b581a2cad88baba97b8b77e3dc0e2ff325d83bffcc513f233f8008ac41"
        );
        let verified = verify_password(
            "correct horse battery",
            &hash,
            "Ada@Example.com",
            "user-id",
            "account-id",
        );
        assert_eq!(verified.ok(), Some(true));
        assert_eq!(stored_hash_rounds(&hash).ok(), Some(rounds));
        let wrong = verify_password("wrong", &hash, "Ada@Example.com", "user-id", "account-id");
        assert_eq!(wrong.ok(), Some(false));
    }

    #[tokio::test]
    async fn a_link_replaced_while_it_is_used_sets_no_password() {
        let (server_context, conn) = crate::test_utils::main_test_db("reset-reinvite").await;
        let account_id = crate::test_utils::insert_test_account(&conn, "reset_reinvite").await;
        let email = format!("p{}@example.com", account_id.replace('-', ""));
        let user_id: String = conn
            .query_one(
                "INSERT INTO users (account_id, name, email, is_admin)
                 VALUES ($1::text::uuid, 'Invitee', $2, FALSE) RETURNING id::text",
                &[&account_id, &email],
            )
            .await
            .expect("insert pending user")
            .get(0);
        let old_token = insert_password_reset_token(&conn, &user_id, 3600)
            .await
            .expect("insert invite token");

        // A re-invite holds the user's row and replaces the link while the
        // invitee submits the old one.
        let reset_token = old_token.to_string();
        let reset = conn
            .in_transaction(async |conn| {
                conn.execute(
                    "SELECT 1 FROM users WHERE id = $1::text::uuid FOR UPDATE",
                    &[&user_id],
                )
                .await?;
                let reset = tokio::spawn(async move {
                    reset_password(
                        server_context,
                        ResetPasswordRequest {
                            token: reset_token,
                            password: "a-long-enough-password".to_string(),
                        },
                    )
                    .await
                });
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                conn.execute(
                    "DELETE FROM password_reset_tokens WHERE user_id = $1::text::uuid",
                    &[&user_id],
                )
                .await?;
                insert_password_reset_token(conn, &user_id, 3600).await?;
                Ok::<_, AuthError>(reset)
            })
            .await
            .expect("re-invite commits");
        let result = reset.await.expect("reset task");
        assert!(
            matches!(result, Err(AuthError::InvalidRequest(_))),
            "got {result:?}"
        );
        let pending: bool = conn
            .query_one(
                "SELECT hashed_password IS NULL FROM users WHERE id = $1::text::uuid",
                &[&user_id],
            )
            .await
            .expect("user")
            .get(0);
        assert!(pending, "the old link must not set a password");

        conn.execute(
            "DELETE FROM accounts WHERE id = $1::text::uuid",
            &[&account_id],
        )
        .await
        .expect("delete account");
    }

    #[test]
    fn signup_timezone_trims_and_falls_back_to_utc() {
        assert_eq!(
            timezone_for_signup(Some(" America/New_York ")),
            "America/New_York"
        );
        assert_eq!(timezone_for_signup(None), "UTC");
        assert_eq!(timezone_for_signup(Some("")), "UTC");
        assert_eq!(timezone_for_signup(Some("   ")), "UTC");
        assert_eq!(timezone_for_signup(Some(&"x".repeat(65))), "UTC");
        // Well-formed but not a real zone, which a length check would accept.
        assert_eq!(timezone_for_signup(Some("America/Nowhere")), "UTC");
    }
}
