//! Starting an import from a connected credential.

use std::sync::Arc;

use jiff::ToSpan;
use jiff::civil::Date;
pub(crate) use revscope_api::ga_import::StartImportRequest;

use crate::auth::AuthError;
use crate::authorize::AdminProject;
use crate::db::{
    open_main_database_read, open_main_database_write, open_project_database_read,
    open_project_database_write,
};
use crate::plan_retention::oldest_retained_date;
use crate::server_context::ServerContext;
use crate::timezone;
use crate::util::{block_on, unix_seconds};

use super::api;
use super::connect::connect_error;
use super::credential::GaCredential;
use super::list_imports::{ImportView, load_import_view};
use super::reports::chunk_count;
use super::store;

pub(super) const MAX_HOSTNAMES: usize = 50;
const MAX_HOSTNAME_LEN: usize = 253;

/// Validates a request's dates against the plan and the project's own tracking.
/// `first_tracked` is the date of the first pageview.
fn validate_range(
    start: Date,
    end: Date,
    today: Date,
    oldest_allowed: Option<Date>,
    first_tracked: Option<Date>,
    allow_duplicates: bool,
) -> Result<(), AuthError> {
    if start > end {
        return Err(AuthError::InvalidRequest(
            "the start date must not be after the end date",
        ));
    }
    if end >= today {
        return Err(AuthError::InvalidRequest(
            "the end date must be before today, since today's numbers aren't final",
        ));
    }
    if oldest_allowed.is_some_and(|oldest| start < oldest) {
        return Err(AuthError::InvalidRequest(
            "the start date is older than your plan's data retention",
        ));
    }
    if !allow_duplicates && first_tracked.is_some_and(|first| end >= first) {
        return Err(AuthError::Conflict(
            "this range overlaps RevScope's own tracking, which would duplicate visits and pageviews",
        ));
    }
    Ok(())
}

fn normalize_hostnames(hostnames: Vec<String>) -> Result<Vec<String>, AuthError> {
    if hostnames.len() > MAX_HOSTNAMES {
        return Err(AuthError::InvalidRequest("too many hostnames"));
    }
    let mut normalized = Vec::with_capacity(hostnames.len());
    for hostname in hostnames {
        let hostname = hostname.trim().to_ascii_lowercase();
        if hostname.is_empty() {
            continue;
        }
        // Tracked hostnames are prefilled here, and we don't constrain those.
        let valid = hostname.len() <= MAX_HOSTNAME_LEN
            && !hostname
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || c == ',');
        if !valid {
            return Err(AuthError::InvalidRequest("invalid hostname"));
        }
        if !normalized.contains(&hostname) {
            normalized.push(hostname);
        }
    }
    Ok(normalized)
}

/// The months of history the project's account keeps, read fresh.
async fn load_retention_months(
    server_context: Arc<ServerContext>,
    project_key: i32,
) -> Result<Option<i64>, AuthError> {
    let conn = open_main_database_read(server_context).await?;
    let row = conn
        .query_opt(
            &format!(
                "SELECT sub.plan_type::text, {}
             FROM projects p
             JOIN account_subscriptions sub
                ON sub.account_id = p.account_id AND sub.ended_at IS NULL
             WHERE p.id = $1",
                crate::plan_retention::RETENTION_STATUS_SQL
            ),
            &[&project_key],
        )
        .await
        .map_err(AuthError::from)?
        .ok_or(AuthError::NotFound("no such project"))?;
    let plan: String = row.get(0);
    let status: Option<String> = row.get(1);
    Ok(crate::plan_retention::account_retention_months(
        &plan,
        status.as_deref(),
    ))
}

pub(crate) fn start_import(
    server_context: Arc<ServerContext>,
    project: &AdminProject,
    request: StartImportRequest,
) -> Result<ImportView, AuthError> {
    // The id goes into the Data API's URL path, so it's checked as well as looked up.
    if request.property_id.is_empty() || !request.property_id.bytes().all(|b| b.is_ascii_digit()) {
        return Err(AuthError::InvalidRequest("invalid property"));
    }
    let parse = |value: &str| {
        value
            .parse::<Date>()
            .map_err(|_| AuthError::InvalidRequest("dates must be YYYY-MM-DD"))
    };
    let start = parse(&request.start_date)?;
    let end = parse(&request.end_date)?;
    let hostnames = normalize_hostnames(request.hostnames)?;

    let timezone = timezone::project_timezone(server_context.clone(), project.key());
    let today = timezone
        .date_of(unix_seconds())
        .ok_or(AuthError::Temporary("couldn't read the current date"))?;
    // Read fresh: this is a hard limit, and a session can hold a plan since changed.
    let retention_months = block_on(load_retention_months(server_context.clone(), project.key()))?;
    let oldest_allowed = oldest_retained_date(retention_months, today);
    let read_conn = open_project_database_read(server_context.as_ref(), project.id())?;
    let first_tracked = store::first_tracked_second(&read_conn)
        .map_err(crate::auth::map_row_error)?
        .and_then(|second| timezone.date_of(second));
    drop(read_conn);
    validate_range(
        start,
        end,
        today,
        oldest_allowed,
        first_tracked,
        request.allow_duplicates,
    )?;

    let project_key = project.key();
    let project_id = project.id().to_string();
    let user_id = project.admin().user().user_id.clone();
    let import_id = block_on(async {
        let conn = open_main_database_write(server_context.clone()).await?;
        let row = conn
            .query_opt(
                "SELECT encrypted_client_email, encrypted_secret, oauth_subject
                 FROM ga_import_connections
                 WHERE id::text = $1
                   AND project_id = $2
                   AND created_by = $3::text::uuid
                   AND expires_at > NOW()
                   AND encrypted_secret IS NOT NULL",
                &[&request.connection_id, &project_key, &user_id],
            )
            .await
            .map_err(AuthError::from)?
            .ok_or(AuthError::InvalidRequest(
                "this Google connection has expired; connect again",
            ))?;
        let email: Option<&[u8]> = row.get(0);
        let secret: &[u8] = row.get(1);
        let subject: Option<&str> = row.get(2);
        let credential = GaCredential::decrypt(email, secret).ok_or(AuthError::InvalidRequest(
            "this Google connection has expired; connect again",
        ))?;

        // Asked again rather than trusted from the form: it proves the credential
        // can read this property, and the name it's listed under comes from Google.
        let token = api::fetch_access_token(&server_context, &credential)
            .await
            .map_err(connect_error)?;
        let properties = api::list_properties(&server_context, &token.access_token)
            .await
            .map_err(connect_error)?;
        let property = properties
            .into_iter()
            .find(|property| property.id == request.property_id)
            .ok_or(AuthError::InvalidRequest(
                "this credential can't read that Google Analytics property",
            ))?;

        let (start_text, end_text) = (start.to_string(), end.to_string());
        let chunks_total = chunk_count(start, end);
        conn.in_transaction(async |conn| {
            // Serializes starts for the project, so two can't both pass the overlap check.
            conn.query_one(
                "SELECT id FROM projects WHERE id = $1 FOR UPDATE",
                &[&project_key],
            )
            .await?;
            let active = conn
                .query_opt(
                    "SELECT 1 FROM ga_imports
                     WHERE project_id = $1 AND status IN ('running', 'paused')",
                    &[&project_key],
                )
                .await?;
            if active.is_some() {
                return Err(AuthError::Conflict(
                    "an import is already running for this project",
                ));
            }
            let overlapping = conn
                .query_opt(
                    "SELECT 1 FROM ga_imports
                     WHERE project_id = $1
                       AND start_date <= $3::text::date
                       AND end_date >= $2::text::date
                     LIMIT 1",
                    &[&project_key, &start_text, &end_text],
                )
                .await?;
            if overlapping.is_some() {
                return Err(AuthError::Conflict(
                    "this range overlaps an earlier import; delete that import or pick other dates",
                ));
            }
            let taken = conn
                .execute(
                    "DELETE FROM ga_import_connections WHERE id::text = $1",
                    &[&request.connection_id],
                )
                .await?;
            if taken == 0 {
                return Err(AuthError::InvalidRequest(
                    "this Google connection has expired; connect again",
                ));
            }
            let email = email.map(<[u8]>::to_vec);
            let secret = secret.to_vec();
            let import_id: i32 = conn
                .query_one(
                    "INSERT INTO ga_imports
                        (project_id, property_id, property_name, start_date, end_date, hostnames,
                         encrypted_client_email, encrypted_secret, oauth_subject, chunks_total)
                     VALUES ($1, $2, $3, $4::text::date, $5::text::date, $6, $7, $8, $9, $10)
                     RETURNING id",
                    &[
                        &project_key,
                        &property.id,
                        &property.name,
                        &start_text,
                        &end_text,
                        &hostnames,
                        &email,
                        &secret,
                        &subject,
                        &chunks_total,
                    ],
                )
                .await?
                .get(0);
            // Written before the commit: if this fails the job never exists, and
            // the worker's writes rely on this row being there.
            let write_conn = open_project_database_write(server_context.as_ref(), &project_id)?;
            store::insert_import_row(
                &write_conn,
                import_id,
                &property.name,
                &start_text,
                &end_text,
            )
            .map_err(crate::auth::map_row_error)?;
            Ok(import_id)
        })
        .await
    })?;
    load_import_view(server_context, project, import_id)
}

/// The day before our own tracking starts, or without tracking the latest day GA
/// can finalize. Tracking older than the default start makes this earlier than that
/// start on purpose: no range avoids duplicates then, so the defaults shouldn't offer
/// one, and the admin has to choose an end date to import anyway.
pub(super) fn default_end_date(today: Date, first_tracked: Option<Date>) -> Option<Date> {
    let yesterday = today.checked_sub(1.day()).ok()?;
    let before_tracking = first_tracked.and_then(|first| first.checked_sub(1.day()).ok());
    Some(before_tracking.map_or(yesterday, |day| day.min(yesterday)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    const TODAY: Date = date(2026, 11, 4);

    fn check(
        start: Date,
        end: Date,
        oldest: Option<Date>,
        first: Option<Date>,
        allow: bool,
    ) -> Result<(), AuthError> {
        validate_range(start, end, TODAY, oldest, first, allow)
    }

    #[test]
    fn a_range_must_be_in_order_and_finished() {
        assert!(check(date(2026, 2, 1), date(2026, 1, 1), None, None, false).is_err());
        assert!(check(date(2026, 1, 1), TODAY, None, None, false).is_err());
        assert!(check(date(2026, 1, 1), date(2026, 11, 3), None, None, false).is_ok());
    }

    #[test]
    fn plan_retention_is_a_hard_limit() {
        let oldest = Some(date(2026, 5, 4));
        assert!(check(date(2026, 5, 3), date(2026, 6, 1), oldest, None, true).is_err());
        assert!(check(date(2026, 5, 4), date(2026, 6, 1), oldest, None, true).is_ok());
    }

    #[test]
    fn overlapping_our_own_tracking_needs_acknowledging() {
        let first = Some(date(2026, 6, 1));
        assert!(check(date(2026, 1, 1), date(2026, 5, 31), None, first, false).is_ok());
        assert!(matches!(
            check(date(2026, 1, 1), date(2026, 6, 1), None, first, false),
            Err(AuthError::Conflict(_))
        ));
        assert!(check(date(2026, 1, 1), date(2026, 6, 1), None, first, true).is_ok());
    }

    #[test]
    fn hostnames_are_normalized_and_checked() {
        assert_eq!(
            normalize_hostnames(vec![
                " ACME.test ".to_string(),
                "acme.test".to_string(),
                "".to_string()
            ])
            .ok(),
            Some(vec!["acme.test".to_string()])
        );
        assert!(normalize_hostnames(vec!["a b".to_string()]).is_err());
        assert!(normalize_hostnames(vec!["my_shop.test".to_string()]).is_ok());
        assert!(normalize_hostnames(vec!["a".to_string(); MAX_HOSTNAMES + 1]).is_err());
    }

    #[test]
    fn the_default_end_stops_short_of_our_own_data() {
        assert_eq!(
            default_end_date(TODAY, Some(date(2026, 8, 10))),
            Some(date(2026, 8, 9))
        );
    }

    #[test]
    fn the_default_end_is_yesterday_without_tracking() {
        assert_eq!(default_end_date(TODAY, None), Some(date(2026, 11, 3)));
    }

    #[test]
    fn the_default_end_never_duplicates_even_when_nothing_can_be_imported() {
        // Tracking predates any recent retention window, and the end still stops short.
        assert_eq!(
            default_end_date(TODAY, Some(date(2025, 1, 1))),
            Some(date(2024, 12, 31))
        );
    }

    #[test]
    fn the_default_end_stays_a_finished_day_whatever_tracking_says() {
        assert_eq!(
            default_end_date(TODAY, Some(TODAY)),
            Some(date(2026, 11, 3))
        );
        // A pageview stamped ahead of the server's clock.
        assert_eq!(
            default_end_date(TODAY, Some(date(2026, 11, 9))),
            Some(date(2026, 11, 3))
        );
    }
}
