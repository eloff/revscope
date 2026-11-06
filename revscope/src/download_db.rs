//! Download a project's SQLite database file.
//!
//! Downloads stream from a consistent copy of the database
//! ([`crate::download_snapshots`]), never the live file, and byte ranges are
//! honoured so an interrupted download can be resumed. A resuming client names
//! the copy it started on with `If-Range` against the returned `ETag`; when
//! that copy is gone it gets the whole of a current one instead.
//!
//! The browser downloads by navigating to the URL, which cannot carry an
//! `Authorization` header, so the dashboard first exchanges its session for a
//! download token that reaches this one endpoint for this one project. Any
//! signed-in account may download the public demo project (`DEMO_PROJECT_ID`),
//! but not anonymously: a download can copy the whole database on disk, which
//! should not be something an unauthenticated caller can trigger at will.

use std::sync::Arc;

use serde::Serialize;
use serde_json::json;

use crate::auth::AuthError;
use crate::authorize::{Account, DownloadableProject};
use crate::db::{open_project_database_read, parse_project_id};
use crate::download_snapshots::Snapshot;
use crate::http::{FileBody, Response};
use crate::server_context::ServerContext;
use crate::tokens::DOWNLOAD_TOKEN_TTL_SECONDS;

const CONTENT_TYPE: &str = "application/vnd.sqlite3";

/// Public demo project; any signed-in account may download its database.
pub(crate) const DEMO_PROJECT_ID: &str = "1";

pub(crate) struct DownloadDbQuery {
    pub(crate) token: Option<String>,
}

/// Parses a `/v2/download/projects/{id}?token=...` target.
pub(crate) fn parse_download_db_query(target: &str) -> DownloadDbQuery {
    DownloadDbQuery {
        token: crate::http::query_param(target, "token").map(str::to_string),
    }
}

#[derive(Serialize)]
pub(crate) struct DownloadTokenResponse {
    token: String,
    /// Roughly what the download will transfer, without copying the database
    /// to find out exactly.
    size_bytes: u64,
    /// Seconds since the unix epoch.
    expires_at: u32,
}

/// What a download token grants: one project's database, for as long as the
/// user who minted it may still read that project. Checked when the token is
/// used, by `authorize::authorize_download_token`.
#[derive(Clone, Debug)]
pub(crate) struct DownloadGrant {
    pub(crate) project_key: i32,
    pub(crate) user_id: String,
    pub(crate) account_id: String,
}

/// Exchanges an authenticated session for a token that grants nothing but
/// downloading this one project's database, and estimates the download's size
/// so the dashboard can show what it will cost.
pub(crate) fn create_download_token(
    server_context: Arc<ServerContext>,
    account: &Account,
    project: &DownloadableProject,
) -> Result<DownloadTokenResponse, AuthError> {
    let project_key = project_key_for_download(&server_context, project.id())?;

    let size_bytes = download_size(&server_context, project.id())?;

    let issued = server_context.download_tokens.issue(
        DownloadGrant {
            project_key,
            user_id: account.user().user_id.clone(),
            account_id: account.user().account_id.clone(),
        },
        DOWNLOAD_TOKEN_TTL_SECONDS,
    )?;

    Ok(DownloadTokenResponse {
        token: issued.secret.to_string(),
        size_bytes,
        expires_at: issued.expires_at,
    })
}

pub(crate) fn download_db(
    server_context: Arc<ServerContext>,
    project: &DownloadableProject,
    range_header: Option<&str>,
    if_range_header: Option<&str>,
) -> Result<Response, AuthError> {
    serve_project_database(server_context, project.id(), range_header, if_range_header)
}

/// Validates that the project has a database file we can serve, returning the
/// numeric key. Parsing also rejects anything that could escape the data
/// directory by way of the file name.
fn project_key_for_download(
    server_context: &Arc<ServerContext>,
    project_id: &str,
) -> Result<i32, AuthError> {
    if server_context.data_dir().is_none() {
        return Err(AuthError::Db(
            "project databases are not stored on disk".to_string(),
        ));
    }
    parse_project_id(project_id).map_err(|error| AuthError::Db(error.to_string()))
}

fn serve_project_database(
    server_context: Arc<ServerContext>,
    project_id: &str,
    range_header: Option<&str>,
    if_range_header: Option<&str>,
) -> Result<Response, AuthError> {
    let project_key = project_key_for_download(&server_context, project_id)?;
    let snapshots = &server_context.download_snapshots;
    // Without `If-Range` a ranged request cannot say which copy its earlier
    // bytes came from, so it gets the whole of a current one rather than a
    // range of a copy that may not match.
    let resuming = match (range_header, if_range_header) {
        (Some(_), Some(validator)) => snapshots.find(project_key, validator.trim()),
        _ => None,
    };
    if let Some(snapshot) = resuming {
        let requested = resolve_range(range_header, snapshot.length);
        return Ok(range_response(project_id, snapshot, requested));
    }
    let snapshot = snapshots
        .current(&server_context, project_id, project_key)
        .map_err(AuthError::Db)?;
    Ok(range_response(project_id, snapshot, Requested::Full))
}

/// Roughly how large a fresh snapshot will be: the pages in use, counting
/// those so far only in the WAL. `VACUUM INTO` drops free pages and repacks
/// the rest, so the copy can come out somewhat smaller.
fn download_size(server_context: &Arc<ServerContext>, project_id: &str) -> Result<u64, AuthError> {
    let conn = open_project_database_read(server_context.as_ref(), project_id)
        .map_err(|error| AuthError::Db(error.to_string()))?;
    // Not `prepare_cached`: a pragma would take a slot in the pooled statement
    // cache that the dashboard queries need.
    let raw: &rusqlite::Connection = &conn;
    let pragma = |name: &str| {
        raw.query_row(&format!("PRAGMA {name}"), [], |row| row.get::<_, i64>(0))
            .map_err(|error| AuthError::Db(error.to_string()))
    };
    let pages = pragma("page_count")? - pragma("freelist_count")?;
    Ok((pages * pragma("page_size")?).unsigned_abs())
}

/// What the client asked for, resolved against the snapshot's length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Requested {
    Full,
    Partial { start: u64, end_inclusive: u64 },
    Unsatisfiable,
}

/// Parses a `Range` header. Only single byte ranges are supported; a malformed
/// or multi-range header falls back to sending the whole file, which the spec
/// allows and which keeps resumption working for the clients that matter.
fn resolve_range(range_header: Option<&str>, length: u64) -> Requested {
    let Some(header) = range_header else {
        return Requested::Full;
    };
    let Some(spec) = strip_bytes_unit(header.trim()) else {
        return Requested::Full;
    };
    if spec.contains(',') {
        return Requested::Full;
    }
    let Some((first, last)) = spec.split_once('-') else {
        return Requested::Full;
    };
    let (first, last) = (first.trim(), last.trim());

    if first.is_empty() {
        // `bytes=-N`: the final N bytes.
        let Ok(suffix_length) = last.parse::<u64>() else {
            return Requested::Full;
        };
        if suffix_length == 0 || length == 0 {
            return Requested::Unsatisfiable;
        }
        return Requested::Partial {
            start: length.saturating_sub(suffix_length),
            end_inclusive: length - 1,
        };
    }

    let Ok(start) = first.parse::<u64>() else {
        return Requested::Full;
    };
    let end_inclusive = if last.is_empty() {
        length.saturating_sub(1)
    } else {
        match last.parse::<u64>() {
            Ok(end) => end.min(length.saturating_sub(1)),
            Err(_) => return Requested::Full,
        }
    };
    if start >= length {
        return Requested::Unsatisfiable;
    }
    if start > end_inclusive {
        return Requested::Full;
    }
    Requested::Partial {
        start,
        end_inclusive,
    }
}

fn strip_bytes_unit(header: &str) -> Option<&str> {
    let (unit, spec) = header.split_once('=')?;
    unit.trim().eq_ignore_ascii_case("bytes").then_some(spec)
}

fn range_response(project_id: &str, snapshot: Arc<Snapshot>, range: Requested) -> Response {
    let length = snapshot.length;
    let (status, offset, body_length, content_range) = match range {
        Requested::Unsatisfiable => {
            return Response::json_with_headers(
                "416 Range Not Satisfiable",
                json!({ "error": "requested range not satisfiable" }).to_string(),
                &[
                    ("Accept-Ranges", "bytes".to_string()),
                    ("Content-Range", format!("bytes */{length}")),
                ],
            );
        }
        Requested::Full => ("200 OK", 0, length, None),
        Requested::Partial {
            start,
            end_inclusive,
        } => (
            "206 Partial Content",
            start,
            end_inclusive - start + 1,
            Some(format!("bytes {start}-{end_inclusive}/{length}")),
        ),
    };

    let mut headers = download_headers(project_id, &snapshot.etag);
    if let Some(content_range) = content_range {
        headers.push(("Content-Range", content_range));
    }
    Response::file(
        status,
        FileBody {
            path: snapshot.path.clone(),
            offset,
            length: body_length,
            _keep_alive: Some(crate::download_snapshots::lease(snapshot)),
        },
        &headers,
    )
}

fn download_headers(project_id: &str, etag: &str) -> Vec<(&'static str, String)> {
    vec![
        ("Content-Type", CONTENT_TYPE.to_string()),
        (
            "Content-Disposition",
            format!("attachment; filename=\"revscope-project-{project_id}.db\""),
        ),
        ("Accept-Ranges", "bytes".to_string()),
        ("ETag", etag.to_string()),
        ("Cache-Control", "no-store".to_string()),
    ]
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::download_snapshots::tests::snapshot_at;
    use crate::test_utils::TestDataDir;
    use crate::tokens::TokenSecret;
    use crate::util::seconds_since_epoch;

    const LENGTH: u64 = 1_000;

    /// A context with project 1's database on disk, plus a live token for it.
    fn context_with_token() -> (Arc<ServerContext>, TestDataDir, String) {
        let data_dir = TestDataDir::new("download-db");
        let server_context = Arc::new(ServerContext::new(data_dir.path().to_path_buf()));
        crate::db::insert_project(server_context.as_ref(), 1, "demo".to_string(), 1);
        crate::db::create_project_database(server_context.as_ref(), "1")
            .expect("create project database");
        let issued = server_context
            .download_tokens
            .issue(demo_grant(), DOWNLOAD_TOKEN_TTL_SECONDS)
            .expect("issue download token");
        (server_context, data_dir, issued.secret.to_string())
    }

    /// Project 1 is the demo, so redeeming its grant needs no holder lookup
    /// in Postgres.
    fn demo_grant() -> DownloadGrant {
        DownloadGrant {
            project_key: 1,
            user_id: "00000000-0000-7000-8000-000000000001".to_string(),
            account_id: "00000000-0000-0000-0000-000000000001".to_string(),
        }
    }

    #[test]
    fn only_the_demo_by_id_skips_checking_the_tokens_holder() {
        crate::test_env::load_e2e_env();
        let (server_context, _data_dir, _token) = context_with_token();
        // Named like the demo, but not the demo, and minted by nobody on it.
        crate::db::insert_project(server_context.as_ref(), 2, "demo".to_string(), 1);
        crate::db::create_project_database(server_context.as_ref(), "2")
            .expect("create project database");
        let grant = DownloadGrant {
            project_key: 2,
            ..demo_grant()
        };
        let issued = server_context
            .download_tokens
            .issue(grant, DOWNLOAD_TOKEN_TTL_SECONDS)
            .expect("issue download token");
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let _guard = runtime.enter();
        let result =
            download_with_token(server_context, &issued.secret.to_string(), "2", None, None);
        assert!(
            matches!(result, Err(AuthError::InvalidSession)),
            "expected the holder lookup to refuse, got {:?}",
            result.map(|_| ())
        );
    }

    fn write_rows(server_context: &ServerContext, rows: &[i64]) {
        let conn = crate::db::open_project_database_write(server_context, "1").expect("write");
        let raw: &rusqlite::Connection = &conn;
        raw.execute("CREATE TABLE IF NOT EXISTS t (x INTEGER)", [])
            .expect("create table");
        for row in rows {
            raw.execute("INSERT INTO t (x) VALUES (?1)", [row])
                .expect("insert");
        }
    }

    fn header<'a>(headers: &'a str, name: &str) -> &'a str {
        headers
            .split("\r\n")
            .find_map(|line| line.strip_prefix(name)?.strip_prefix(": "))
            .unwrap_or_else(|| panic!("no {name} header in {headers}"))
    }

    fn rows_in(path: &std::path::Path) -> Vec<i64> {
        let conn =
            rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .expect("open snapshot");
        let integrity: String = conn
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .expect("integrity check");
        assert_eq!(integrity, "ok");
        let mut statement = conn.prepare("SELECT x FROM t ORDER BY x").expect("prepare");
        statement
            .query_map([], |row| row.get(0))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("rows")
    }

    /// What the router does for a token-carrying download: grant, then serve.
    fn download_with_token(
        server_context: Arc<ServerContext>,
        token: &str,
        project_id: &str,
        range_header: Option<&str>,
        if_range_header: Option<&str>,
    ) -> Result<Response, AuthError> {
        let project =
            crate::authorize::authorize_download_token(server_context.clone(), token, project_id)?;
        download_db(server_context, &project, range_header, if_range_header)
    }

    #[test]
    fn a_download_is_a_copy_that_later_writes_do_not_touch() {
        let (server_context, _data_dir, token) = context_with_token();
        write_rows(&server_context, &[1, 2]);
        let response =
            download_with_token(server_context.clone(), &token, "1", None, None).expect("download");
        let file_body = response.file_body.expect("file body");
        let live = crate::db::project_database_path(server_context.as_ref(), "1");
        assert_ne!(file_body.path, live);

        write_rows(&server_context, &[3]);
        assert_eq!(rows_in(&file_body.path), vec![1, 2]);
        assert_eq!(
            file_body.length,
            std::fs::metadata(&file_body.path).expect("metadata").len()
        );
    }

    #[test]
    fn a_resume_names_its_copy_with_if_range_and_gets_the_rest_of_it() {
        let (server_context, _data_dir, token) = context_with_token();
        write_rows(&server_context, &[1]);
        let first =
            download_with_token(server_context.clone(), &token, "1", None, None).expect("download");
        let headers = String::from_utf8(first.headers).expect("utf8");
        let etag = header(&headers, "ETag").to_string();
        let first_body = first.file_body.expect("file body");

        write_rows(&server_context, &[2]);
        let resumed = download_with_token(
            server_context.clone(),
            &token,
            "1",
            Some("bytes=100-"),
            Some(&etag),
        )
        .expect("resume");
        let headers = String::from_utf8(resumed.headers).expect("utf8");
        assert!(headers.starts_with("HTTP/1.1 206 Partial Content\r\n"));
        assert_eq!(header(&headers, "ETag"), etag);
        let resumed_body = resumed.file_body.expect("file body");
        assert_eq!(resumed_body.path, first_body.path);
        assert_eq!(resumed_body.offset, 100);
        assert_eq!(resumed_body.length, first_body.length - 100);
    }

    #[test]
    fn a_range_for_an_unknown_or_unnamed_copy_gets_the_whole_of_a_current_one() {
        let (server_context, _data_dir, token) = context_with_token();
        for if_range in [Some("\"gone\""), None] {
            let response = download_with_token(
                server_context.clone(),
                &token,
                "1",
                Some("bytes=100-"),
                if_range,
            )
            .expect("download");
            let headers = String::from_utf8(response.headers).expect("utf8");
            assert!(headers.starts_with("HTTP/1.1 200 OK\r\n"), "{headers}");
            assert_eq!(response.file_body.expect("file body").offset, 0);
        }
    }

    #[test]
    fn deleting_the_project_deletes_its_copies_once_no_response_streams_them() {
        let (server_context, _data_dir, token) = context_with_token();
        let response =
            download_with_token(server_context.clone(), &token, "1", None, None).expect("download");
        let path = response.file_body.as_ref().expect("file body").path.clone();
        crate::db::remove_project_database(server_context.as_ref(), "1").expect("remove");
        assert!(path.exists(), "the response still has to write it");
        drop(response);
        assert!(!path.exists());
    }

    #[test]
    fn a_copy_finished_after_the_project_was_deleted_is_discarded() {
        let (server_context, data_dir, token) = context_with_token();
        server_context.deleted_project_ids.insert(1);
        let result = download_with_token(server_context.clone(), &token, "1", None, None);
        assert!(matches!(result, Err(AuthError::Db(_))));
        assert!(!server_context.download_snapshots.has_project(1));
        let own = data_dir
            .path()
            .join("download-snapshots")
            .join(std::process::id().to_string());
        let left: Vec<_> = std::fs::read_dir(own).expect("snapshot dir").collect();
        assert!(left.is_empty(), "{left:?}");
    }

    fn snapshot(length: u64) -> Arc<Snapshot> {
        Arc::new(snapshot_at(
            PathBuf::from("/nonexistent/revscope-download-test.db"),
            length,
            "\"abc\"",
        ))
    }

    #[test]
    fn a_live_token_downloads_the_project_it_was_issued_for() {
        let (server_context, _data_dir, token) = context_with_token();
        let response =
            download_with_token(server_context, &token, "1", None, None).expect("download");
        let headers = String::from_utf8(response.headers).expect("utf8");
        assert!(headers.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(response.file_body.is_some());
    }

    #[test]
    fn a_token_is_refused_for_another_project_and_after_it_expires() {
        let (server_context, _data_dir, token) = context_with_token();
        assert!(matches!(
            download_with_token(server_context.clone(), &token, "2", None, None),
            Err(AuthError::InvalidSession)
        ));
        assert!(matches!(
            download_with_token(server_context.clone(), "not-a-token", "1", None, None),
            Err(AuthError::InvalidSession)
        ));

        let secret = TokenSecret::parse(&token).expect("token");
        server_context
            .download_tokens
            .insert(secret, demo_grant(), seconds_since_epoch() - 1);
        assert!(matches!(
            download_with_token(server_context.clone(), &token, "1", None, None),
            Err(AuthError::InvalidSession)
        ));
        assert!(server_context.download_tokens.is_empty());
    }

    fn partial(start: u64, end_inclusive: u64) -> Requested {
        Requested::Partial {
            start,
            end_inclusive,
        }
    }

    #[test]
    fn parses_the_token_from_the_query_string() {
        let query = parse_download_db_query("/v2/download/projects/1?token=abc");
        assert_eq!(query.token.as_deref(), Some("abc"));
        assert_eq!(
            parse_download_db_query("/v2/download/projects/1").token,
            None
        );
        assert_eq!(
            parse_download_db_query("/v2/download/projects/1?token=").token,
            None
        );
    }

    #[test]
    fn resolves_open_and_closed_byte_ranges() {
        assert_eq!(resolve_range(Some("bytes=0-"), LENGTH), partial(0, 999));
        assert_eq!(resolve_range(Some("bytes=500-"), LENGTH), partial(500, 999));
        assert_eq!(
            resolve_range(Some("bytes=100-199"), LENGTH),
            partial(100, 199)
        );
        assert_eq!(resolve_range(Some("bytes=-100"), LENGTH), partial(900, 999));
        assert_eq!(
            resolve_range(Some("BYTES = 10 - 20 "), LENGTH),
            partial(10, 20)
        );
    }

    #[test]
    fn clamps_ends_past_the_file_and_rejects_starts_past_it() {
        assert_eq!(
            resolve_range(Some("bytes=900-100000"), LENGTH),
            partial(900, 999)
        );
        assert_eq!(
            resolve_range(Some("bytes=-100000"), LENGTH),
            partial(0, 999)
        );
        assert_eq!(
            resolve_range(Some("bytes=1000-"), LENGTH),
            Requested::Unsatisfiable
        );
        assert_eq!(resolve_range(Some("bytes=0-"), 0), Requested::Unsatisfiable);
    }

    #[test]
    fn falls_back_to_the_whole_file_for_unsupported_headers() {
        assert_eq!(resolve_range(None, LENGTH), Requested::Full);
        assert_eq!(resolve_range(Some("items=0-1"), LENGTH), Requested::Full);
        assert_eq!(
            resolve_range(Some("bytes=0-1,5-6"), LENGTH),
            Requested::Full
        );
        assert_eq!(resolve_range(Some("bytes=abc-"), LENGTH), Requested::Full);
        assert_eq!(
            resolve_range(Some("bytes=500-100"), LENGTH),
            Requested::Full
        );
        assert_eq!(resolve_range(Some("bytes"), LENGTH), Requested::Full);
    }

    #[test]
    fn partial_response_advertises_the_served_range() {
        let response = range_response("1", snapshot(LENGTH), partial(100, 199));
        let headers = String::from_utf8(response.headers).expect("utf8");
        assert!(headers.starts_with("HTTP/1.1 206 Partial Content\r\n"));
        assert!(headers.contains("Content-Range: bytes 100-199/1000\r\n"));
        assert!(headers.contains("Content-Length: 100\r\n"));
        assert!(headers.contains("Accept-Ranges: bytes\r\n"));
        assert!(headers.contains("ETag: \"abc\"\r\n"));
        assert!(
            headers.contains(
                "Content-Disposition: attachment; filename=\"revscope-project-1.db\"\r\n"
            )
        );
        let file_body = response.file_body.expect("file body");
        assert_eq!(file_body.offset, 100);
        assert_eq!(file_body.length, 100);
    }

    #[test]
    fn full_response_sends_the_whole_file_without_a_content_range() {
        let response = range_response("1", snapshot(LENGTH), Requested::Full);
        let headers = String::from_utf8(response.headers).expect("utf8");
        assert!(headers.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(!headers.contains("Content-Range"));
        assert!(headers.contains("Content-Length: 1000\r\n"));
        let file_body = response.file_body.expect("file body");
        assert_eq!(file_body.offset, 0);
        assert_eq!(file_body.length, LENGTH);
    }

    #[test]
    fn unsatisfiable_response_reports_the_current_length() {
        let response = range_response("1", snapshot(LENGTH), Requested::Unsatisfiable);
        let headers = String::from_utf8(response.headers).expect("utf8");
        assert!(headers.starts_with("HTTP/1.1 416 Range Not Satisfiable\r\n"));
        assert!(headers.contains("Content-Range: bytes */1000\r\n"));
        assert!(response.file_body.is_none());
    }
}
