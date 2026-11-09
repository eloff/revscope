// Hosted signup makes the account's first admin, and hosted plans have a retention.
#![cfg(not(feature = "self-hosted"))]

//! Importing from Google Analytics against a stub of Google: connecting, the job
//! running to completion, what reports then show, and removing it again.

mod common;

use std::io;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use common::{HttpResponse, TestServer};
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const SIGNUP_PATH: &str = "/v2/public/auth/signup";
const PASSWORD: &str = "a-long-enough-password";

const MODE_OK: u8 = 0;
const MODE_QUOTA: u8 = 1;
const MODE_DENIED: u8 = 2;

/// Stands in for Google's token, admin and data endpoints.
struct StubGoogle {
    base_url: String,
    mode: Arc<AtomicU8>,
    revoked: Arc<Mutex<Vec<String>>>,
}

impl StubGoogle {
    async fn start() -> io::Result<Self> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base_url = format!("http://{}", listener.local_addr()?);
        let mode = Arc::new(AtomicU8::new(MODE_OK));
        let revoked = Arc::new(Mutex::new(Vec::new()));
        let (task_mode, task_revoked) = (mode.clone(), revoked.clone());
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(serve(stream, task_mode.clone(), task_revoked.clone()));
            }
        });
        Ok(Self {
            base_url,
            mode,
            revoked,
        })
    }
}

async fn serve(
    mut stream: tokio::net::TcpStream,
    mode: Arc<AtomicU8>,
    revoked: Arc<Mutex<Vec<String>>>,
) {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 8192];
    let head_end = loop {
        if let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break end + 4;
        }
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(read) => buffer.extend_from_slice(&chunk[..read]),
        }
    };
    let head = String::from_utf8_lossy(&buffer[..head_end]).into_owned();
    let content_length = head
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    while buffer.len() < head_end + content_length {
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(read) => buffer.extend_from_slice(&chunk[..read]),
        }
    }
    let target = head.split(' ').nth(1).unwrap_or_default().to_string();
    let body = String::from_utf8_lossy(&buffer[head_end..head_end + content_length]).into_owned();
    let (status, json) = respond(&target, &body, mode.load(Ordering::SeqCst), &revoked);
    let body = json.to_string();
    let response = format!(
        "HTTP/1.1 {status} Stub\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// A fresh Google account each run, since an earlier failed run can leave credentials
/// for its account in the shared database.
fn run_subject() -> &'static str {
    static SUBJECT: OnceLock<String> = OnceLock::new();
    SUBJECT.get_or_init(|| {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        format!("google-{nanos}")
    })
}

/// Unsigned, as the client doesn't verify it.
fn id_token() -> String {
    let payload = URL_SAFE_NO_PAD.encode(json!({ "sub": run_subject() }).to_string());
    format!("e30.{payload}.sig")
}

fn respond(target: &str, body: &str, mode: u8, revoked: &Mutex<Vec<String>>) -> (u16, Value) {
    if target.starts_with("/token") {
        return (
            200,
            json!({
                "access_token": "access",
                "expires_in": 3600,
                "refresh_token": format!("refresh-{}", run_subject()),
                "id_token": id_token(),
            }),
        );
    }
    if target.starts_with("/revoke") {
        // The worker sweeps every expired connection in the shared database, so
        // leftovers from earlier runs can land here too.
        if body.contains(run_subject()) {
            revoked.lock().push(body.to_string());
        }
        return (200, json!({}));
    }
    if target.starts_with("/admin/accountSummaries") {
        // Two pages, so a client reading only the first misses the property it wants.
        return if target.contains("pageToken=next") {
            (
                200,
                json!({ "accountSummaries": [{
                    "displayName": "Acme",
                    "propertySummaries": [{ "property": "properties/123", "displayName": "Acme Web" }],
                }] }),
            )
        } else {
            (
                200,
                json!({
                    "accountSummaries": [{
                        "displayName": "Other",
                        "propertySummaries": [{ "property": "properties/9", "displayName": "Other" }],
                    }],
                    "nextPageToken": "next",
                }),
            )
        };
    }
    if target.contains(":runReport") {
        match mode {
            MODE_QUOTA => return (429, json!({ "error": { "status": "RESOURCE_EXHAUSTED" } })),
            MODE_DENIED => return (403, json!({ "error": { "status": "PERMISSION_DENIED" } })),
            _ => {}
        }
        let request: Value = serde_json::from_str(body).expect("report request");
        return (200, report(&request));
    }
    (404, json!({ "error": "unknown route" }))
}

/// The same rows for any report on the import's one day, keyed by its dimensions.
fn report(request: &Value) -> Value {
    let dimensions: Vec<&str> = request["dimensions"]
        .as_array()
        .expect("dimensions")
        .iter()
        .map(|dimension| dimension["name"].as_str().expect("name"))
        .collect();
    let day = request["dateRanges"][0]["endDate"]
        .as_str()
        .expect("end date")
        .replace('-', "");
    let rows: Vec<(Vec<&str>, Vec<&str>)> = match dimensions.get(1).copied() {
        None => vec![(vec![], vec!["100", "300", "60", "5000"])],
        Some("hostName") if dimensions[2] == "pagePath" => {
            vec![
                (vec!["acme.test", "/pricing"], vec!["40", "90", "1800"]),
                (vec!["acme.test", "(other)"], vec!["9", "9", "9"]),
            ]
        }
        Some("hostName") => vec![(vec!["acme.test", "/"], vec!["70", "50", "2800"])],
        Some("sessionSource") => vec![
            (vec!["(direct)", "(none)", "(direct)", "Direct"], vec!["30"]),
            (
                vec!["google", "organic", "(organic)", "Organic Search"],
                vec!["50"],
            ),
            (vec!["newsletter", "email", "launch", "Email"], vec!["20"]),
        ],
        Some("countryId") => vec![(vec!["US", "New York", "Albany"], vec!["100"])],
        Some("deviceCategory") => vec![(vec!["mobile", "Chrome", "Android"], vec!["100"])],
        Some("eventName") => vec![(vec!["signup", "acme.test", "/pricing"], vec!["5", "7"])],
        Some(other) => panic!("unexpected dimension {other}"),
    };
    let rows: Vec<Value> = rows
        .into_iter()
        .map(|(values, metrics)| {
            let mut dimension_values = vec![json!({ "value": day })];
            dimension_values.extend(values.iter().map(|value| json!({ "value": value })));
            json!({
                "dimensionValues": dimension_values,
                "metricValues": metrics.iter().map(|value| json!({ "value": value })).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({ "rowCount": rows.len(), "rows": rows })
}

struct Account {
    project_id: String,
    cookie: String,
    email: String,
}

async fn signup(server: &TestServer) -> io::Result<Account> {
    let slug = format!(
        "gaimport{:x}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_micros()
    );
    let project_name = format!("{slug}example");
    let response = server
        .post_json(
            SIGNUP_PATH,
            &json!({
                "name": "Ada Lovelace",
                "account_name": format!("{slug}.example"),
                "email": format!("{slug}@example.com"),
                "password": PASSWORD,
                "plan": "growth",
                "events": "100k",
                "billing": "monthly",
            })
            .to_string(),
            &[],
        )
        .await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    common::signup::mark_subscription_paid(server, &format!("{slug}.example")).await;
    let project_id = response.json::<Value>()["projects"][&project_name]["id"]
        .as_str()
        .expect("project id")
        .to_string();
    let cookie = response
        .header("set-cookie")
        .expect("session cookie")
        .split(';')
        .next()
        .expect("cookie pair")
        .to_string();
    Ok(Account {
        project_id,
        cookie,
        email: format!("{slug}@example.com"),
    })
}

impl Account {
    /// A new session, since the current one keeps its role and plan in the session cache.
    async fn log_in_again(&mut self, server: &TestServer) -> io::Result<()> {
        let response = server
            .post_json(
                "/v2/public/auth/login",
                &json!({ "username": self.email, "password": PASSWORD }).to_string(),
                &[],
            )
            .await?;
        assert_eq!(response.status_code(), 200, "{}", response.body_str());
        self.cookie = response
            .header("set-cookie")
            .expect("session cookie")
            .split(';')
            .next()
            .expect("cookie pair")
            .to_string();
        Ok(())
    }

    fn path(&self, rest: &str) -> String {
        format!("/v2/session/projects/{}/ga-imports{rest}", self.project_id)
    }

    fn read_path(&self, rest: &str) -> String {
        format!("/v2/read/projects/{}/{rest}", self.project_id)
    }

    async fn post(&self, server: &TestServer, path: &str, body: Value) -> io::Result<HttpResponse> {
        server
            .post_json(path, &body.to_string(), &[("Cookie", self.cookie.as_str())])
            .await
    }

    async fn get(&self, server: &TestServer, path: &str) -> io::Result<Value> {
        let response = server
            .get_query(path, &[("Cookie", self.cookie.as_str())])
            .await?;
        assert_eq!(
            response.status_code(),
            200,
            "{path}: {}",
            response.body_str()
        );
        Ok(response.json())
    }

    /// Connects through the OAuth route and returns the connection id.
    async fn connect(&self, server: &TestServer) -> io::Result<String> {
        let url = self
            .post(server, &self.path("/oauth-url"), json!({}))
            .await?;
        assert_eq!(url.status_code(), 200, "{}", url.body_str());
        let url = url.json::<Value>()["url"]
            .as_str()
            .expect("url")
            .to_string();
        let state = url
            .split("state=")
            .nth(1)
            .expect("state in the url")
            .split('&')
            .next()
            .expect("state")
            .to_string();
        assert!(url.contains("access_type=offline"), "{url}");
        assert!(url.contains("analytics.readonly"), "{url}");
        let connected = self
            .post(
                server,
                &self.path("/connect"),
                json!({ "oauth_code": "code", "oauth_state": state }),
            )
            .await?;
        assert_eq!(connected.status_code(), 200, "{}", connected.body_str());
        let connected: Value = connected.json();
        assert!(
            connected["properties"]
                .as_array()
                .expect("properties")
                .iter()
                .any(|property| property["id"] == "123"),
            "properties on the second page are listed: {connected}"
        );

        // The state is single use.
        let again = self
            .post(
                server,
                &self.path("/connect"),
                json!({ "oauth_code": "code", "oauth_state": state }),
            )
            .await?;
        assert_eq!(again.status_code(), 400, "{}", again.body_str());
        Ok(connected["connection_id"]
            .as_str()
            .expect("connection id")
            .to_string())
    }

    async fn start(
        &self,
        server: &TestServer,
        connection_id: &str,
        date: &str,
    ) -> io::Result<HttpResponse> {
        self.post(
            server,
            &self.path(""),
            json!({
                "connection_id": connection_id,
                "property_id": "123",
                "start_date": date,
                "end_date": date,
            }),
        )
        .await
    }

    async fn status_of(&self, server: &TestServer, import_id: i64) -> io::Result<Value> {
        let listed = self.get(server, &self.path("")).await?;
        Ok(listed["imports"]
            .as_array()
            .expect("imports")
            .iter()
            .find(|import| import["id"] == import_id)
            .cloned()
            .unwrap_or(Value::Null))
    }

    async fn wait_for_status(
        &self,
        server: &TestServer,
        import_id: i64,
        wanted: &str,
    ) -> io::Result<Value> {
        for _ in 0..400 {
            let import = self.status_of(server, import_id).await?;
            if import["status"] == wanted {
                return Ok(import);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!(
            "import {import_id} never became {wanted}: {}",
            self.status_of(server, import_id).await?
        );
    }
}

fn range_query(date_start: i64) -> String {
    format!("?start={date_start}&end={}", date_start + 86_400)
}

#[tokio::test]
async fn an_import_runs_shows_in_reports_and_can_be_removed() -> io::Result<()> {
    // Safe: this binary's only test, and it hasn't started any other threads' work yet.
    unsafe { std::env::set_var("INTEGRATION_ENCRYPTION_KEY", "11".repeat(32)) };
    let google = StubGoogle::start().await?;
    let server = TestServer::spawn_with_ga_api("ga-import", &google.base_url).await?;
    revscope::start_ga_import_task(server.context.clone());
    let account = signup(&server).await?;

    // A day inside every plan's retention, comfortably in the past.
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_secs() as i64;
    let day_start = (now / 86_400 - 30) * 86_400;
    let date = jiff::Timestamp::from_second(day_start)
        .expect("timestamp")
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date()
        .to_string();

    let defaults = account.get(&server, &account.path("")).await?;
    assert_eq!(defaults["oauth_available"], json!(true));
    assert_eq!(defaults["include_in_reports"], json!(true));
    assert_eq!(defaults["first_tracked_date"], Value::Null);
    assert_eq!(
        defaults["default_end_date"],
        defaults["latest_allowed_date"]
    );

    // A pasted key must be one.
    let bad_key = account
        .post(
            &server,
            &account.path("/connect"),
            json!({ "service_account_json": "{\"client_email\":\"a@b\"}" }),
        )
        .await?;
    assert_eq!(bad_key.status_code(), 400, "{}", bad_key.body_str());

    let connection_id = account.connect(&server).await?;

    // Asking for a property the credential can't read is refused.
    let wrong_property = account
        .post(
            &server,
            &account.path(""),
            json!({
                "connection_id": connection_id,
                "property_id": "555",
                "start_date": date,
                "end_date": date,
            }),
        )
        .await?;
    assert_eq!(
        wrong_property.status_code(),
        400,
        "{}",
        wrong_property.body_str()
    );

    let started = account.start(&server, &connection_id, &date).await?;
    assert_eq!(started.status_code(), 200, "{}", started.body_str());
    let import_id = started.json::<Value>()["id"].as_i64().expect("id");
    let finished = account
        .wait_for_status(&server, import_id, "completed")
        .await?;
    assert_eq!(finished["sessions_imported"], json!(100));
    assert_eq!(finished["chunks_done"], finished["chunks_total"]);

    let conn = common::signup::main_db(&server).await;
    let credentials = conn
        .query_one(
            "SELECT encrypted_secret IS NULL, encrypted_client_email IS NULL FROM ga_imports WHERE id = $1",
            &[&(import_id as i32)],
        )
        .await
        .expect("import row");
    assert!(credentials.get::<_, bool>(0), "the refresh token was kept");
    assert_eq!(
        google.revoked.lock().len(),
        1,
        "the refresh token wasn't revoked"
    );

    let query = range_query(day_start);
    let overview = account
        .get(
            &server,
            &format!("{}{query}", account.read_path("overview")),
        )
        .await?;
    assert_eq!(overview["current_metrics"]["visitors"], json!(100.0));
    assert_eq!(overview["current_metrics"]["bounce_rate"], json!(0.4));
    let sources = overview["breakdowns"]["sources"]
        .as_array()
        .expect("sources");
    let source_label = |label: &str| {
        sources
            .iter()
            .find(|row| row["label"] == label)
            .map(|row| row["visitors"].clone())
    };
    assert_eq!(source_label("google.com"), Some(json!(50)));
    assert_eq!(source_label("Direct"), Some(json!(30)));
    assert_eq!(source_label("launch"), Some(json!(20)));
    assert_eq!(overview["breakdowns"]["countries"][0]["value"], json!("US"));
    assert_eq!(
        overview["breakdowns"]["cities"][0]["value"],
        json!("US-Albany")
    );
    assert_eq!(
        overview["breakdowns"]["devices"][0]["value"],
        json!("mobile")
    );
    assert_eq!(overview["breakdowns"]["os"][0]["value"], json!("android"));
    assert_eq!(
        overview["breakdowns"]["pages"][0]["label"],
        json!("/pricing")
    );
    assert_eq!(
        overview["breakdowns"]["landing_pages"][0]["label"],
        json!("/")
    );

    let pages = account
        .get(&server, &format!("{}{query}", account.read_path("pages")))
        .await?;
    assert_eq!(pages["pages"][0]["path"], json!("/pricing"));
    assert_eq!(pages["pages"][0]["visitors"], json!(40));
    // No tracked sessions to compute a page's bounce rate from, so imported days stay out of it.
    assert_eq!(pages["pages"][0]["bounce_rate"], json!(0.0));
    assert_eq!(
        pages["pages"].as_array().map(Vec::len),
        Some(1),
        "(other) isn't a page"
    );

    let landing = account
        .get(
            &server,
            &format!("{}{query}", account.read_path("landing-pages")),
        )
        .await?;
    assert_eq!(landing["pages"][0]["visitors"], json!(70));
    let bounce = landing["pages"][0]["bounce_rate"]
        .as_f64()
        .expect("bounce rate");
    assert!((bounce - 20.0 / 70.0).abs() < 1e-9, "{bounce}");

    let audience = account
        .get(
            &server,
            &format!("{}{query}", account.read_path("audience")),
        )
        .await?;
    assert_eq!(audience["countries"]["current"][0]["visitors"], json!(100));
    assert_eq!(audience["cities"]["current"][0]["city"], json!("Albany"));
    assert_eq!(audience["browsers"]["current"][0]["visitors"], json!(100));

    let events = account
        .get(&server, &format!("{}{query}", account.read_path("events")))
        .await?;
    assert_eq!(events["events"][0]["event_name"], json!("signup"));
    assert_eq!(events["events"][0]["visitors"], json!(5));

    let coverage = account
        .get(
            &server,
            &format!("{}{query}", account.read_path("imported-coverage")),
        )
        .await?;
    assert_eq!(coverage["has_data"], json!(true));
    assert_eq!(coverage["ranges"][0]["start_date"], json!(date));

    // A filter can't be answered from single-dimension rows.
    let filtered = account
        .get(
            &server,
            &format!("{}{query}&country=US", account.read_path("overview")),
        )
        .await?;
    assert_eq!(filtered["current_metrics"]["visitors"], json!(0.0));

    // The setting takes imported data out of reports, and coverage still sees it.
    let off = account
        .post(
            &server,
            &account.path("/settings"),
            json!({ "include_in_reports": false }),
        )
        .await?;
    assert_eq!(off.status_code(), 200, "{}", off.body_str());
    let without = account
        .get(
            &server,
            &format!("{}{query}", account.read_path("overview")),
        )
        .await?;
    assert_eq!(without["current_metrics"]["visitors"], json!(0.0));
    let coverage = account
        .get(
            &server,
            &format!("{}{query}", account.read_path("imported-coverage")),
        )
        .await?;
    assert_eq!(coverage["include_in_reports"], json!(false));
    assert_eq!(coverage["has_data"], json!(true));
    account
        .post(
            &server,
            &account.path("/settings"),
            json!({ "include_in_reports": true }),
        )
        .await?;

    // Dates can't overlap an import that still has rows.
    let connection_id = account.connect(&server).await?;
    let overlap = account.start(&server, &connection_id, &date).await?;
    assert_eq!(overlap.status_code(), 409, "{}", overlap.body_str());

    // Out of quota pauses until the next hour, then resumes by itself.
    let earlier = jiff::civil::Date::strptime("%Y-%m-%d", &date)
        .expect("date")
        .checked_sub(jiff::ToSpan::days(40))
        .expect("earlier")
        .to_string();
    google.mode.store(MODE_QUOTA, Ordering::SeqCst);
    let started = account.start(&server, &connection_id, &earlier).await?;
    assert_eq!(started.status_code(), 200, "{}", started.body_str());
    let paused_id = started.json::<Value>()["id"].as_i64().expect("id");
    account
        .wait_for_status(&server, paused_id, "paused")
        .await?;

    // Only one runs at a time, and a running one can't be deleted.
    let busy = account.connect(&server).await?;
    let elsewhere = jiff::civil::Date::strptime("%Y-%m-%d", &date)
        .expect("date")
        .checked_sub(jiff::ToSpan::days(80))
        .expect("elsewhere")
        .to_string();
    let second = account.start(&server, &busy, &elsewhere).await?;
    assert_eq!(second.status_code(), 409, "{}", second.body_str());
    assert!(
        second.body_str().contains("already running"),
        "{}",
        second.body_str()
    );
    let delete_path = account.path(&format!("?id={paused_id}"));
    let refused = server
        .delete(&delete_path, &[("Cookie", account.cookie.as_str())])
        .await?;
    assert_eq!(refused.status_code(), 409, "{}", refused.body_str());

    google.mode.store(MODE_OK, Ordering::SeqCst);
    conn.execute(
        "UPDATE ga_imports SET resume_at = NOW() WHERE id = $1",
        &[&(paused_id as i32)],
    )
    .await
    .expect("skip the wait");
    account
        .wait_for_status(&server, paused_id, "completed")
        .await?;

    // Access taken away ends the job with a reason and no stored credential.
    let later = jiff::civil::Date::strptime("%Y-%m-%d", &date)
        .expect("date")
        .checked_add(jiff::ToSpan::days(1))
        .expect("later")
        .to_string();
    google.mode.store(MODE_DENIED, Ordering::SeqCst);
    let started = account.start(&server, &busy, &later).await?;
    assert_eq!(started.status_code(), 200, "{}", started.body_str());
    let denied_id = started.json::<Value>()["id"].as_i64().expect("id");
    let failed = account
        .wait_for_status(&server, denied_id, "failed")
        .await?;
    assert!(
        failed["error"]
            .as_str()
            .expect("error")
            .contains("refused access"),
        "{failed}"
    );
    google.mode.store(MODE_OK, Ordering::SeqCst);

    // Deleting gives the reports and the dates back.
    let deleted = server
        .delete(
            &account.path(&format!("?id={import_id}")),
            &[("Cookie", account.cookie.as_str())],
        )
        .await?;
    assert_eq!(deleted.status_code(), 200, "{}", deleted.body_str());
    let after = account
        .get(
            &server,
            &format!("{}{query}", account.read_path("overview")),
        )
        .await?;
    assert_eq!(after["current_metrics"]["visitors"], json!(0.0));
    let coverage = account
        .get(
            &server,
            &format!("{}{query}", account.read_path("imported-coverage")),
        )
        .await?;
    assert_eq!(coverage["has_data"], json!(false));
    assert_eq!(coverage["ranges"], json!([]));

    // Revoking ends the Google account's whole grant, so it waits for the last credential.
    google.mode.store(MODE_QUOTA, Ordering::SeqCst);
    let kept = account.connect(&server).await?;
    let cancelling = account.connect(&server).await?;
    let revokes_before = google.revoked.lock().len();
    let started = account.start(&server, &cancelling, &date).await?;
    assert_eq!(started.status_code(), 200, "{}", started.body_str());
    let cancel_id = started.json::<Value>()["id"].as_i64().expect("id");
    account
        .wait_for_status(&server, cancel_id, "paused")
        .await?;
    let cancelled = account
        .post(
            &server,
            &account.path(&format!("/cancel?id={cancel_id}")),
            json!({}),
        )
        .await?;
    assert_eq!(cancelled.status_code(), 200, "{}", cancelled.body_str());
    assert_eq!(cancelled.json::<Value>()["status"], json!("cancelled"));
    assert_eq!(
        google.revoked.lock().len(),
        revokes_before,
        "another connection still needs this grant"
    );
    google.mode.store(MODE_OK, Ordering::SeqCst);
    let again = account
        .post(
            &server,
            &account.path(&format!("/cancel?id={cancel_id}")),
            json!({}),
        )
        .await?;
    assert_eq!(again.status_code(), 409, "{}", again.body_str());
    conn.execute(
        "UPDATE ga_import_connections SET expires_at = NOW() WHERE id::text = $1",
        &[&kept],
    )
    .await
    .expect("expire the connection");
    for _ in 0..400 {
        if google.revoked.lock().len() > revokes_before {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        google.revoked.lock().len(),
        revokes_before + 1,
        "the last connection going ends the grant"
    );

    // Deleting a project cascades its credentials away, so they're revoked first.
    let added = account
        .post(
            &server,
            "/v2/session/projects",
            json!({ "website": format!("shop{}.example", account.project_id) }),
        )
        .await?;
    assert_eq!(added.status_code(), 200, "{}", added.body_str());
    let added: Value = added.json();
    let doomed = Account {
        project_id: added["id"].as_str().expect("id").to_string(),
        cookie: account.cookie.clone(),
        email: account.email.clone(),
    };
    doomed.connect(&server).await?;
    let revokes_before = google.revoked.lock().len();
    let removed = server
        .delete_json(
            &format!("/v2/session/projects/{}", doomed.project_id),
            &json!({ "confirm_name": added["name"] }).to_string(),
            &[("Cookie", account.cookie.as_str())],
        )
        .await?;
    assert_eq!(removed.status_code(), 200, "{}", removed.body_str());
    assert_eq!(google.revoked.lock().len(), revokes_before + 1);

    // A credential from another project's connection isn't this one's to use.
    let other = signup(&server).await?;
    let connection_id = account.connect(&server).await?;
    let foreign = other.start(&server, &connection_id, &date).await?;
    assert_eq!(foreign.status_code(), 400, "{}", foreign.body_str());
    let listed = server
        .get_query(&account.path(""), &[("Cookie", other.cookie.as_str())])
        .await?;
    assert_ne!(listed.status_code(), 200, "{}", listed.body_str());

    // The demo isn't anyone's to import into.
    let demo = server
        .get_query(
            &format!(
                "/v2/session/projects/{}/ga-imports",
                common::DEMO_PROJECT_ID
            ),
            &[("Cookie", account.cookie.as_str())],
        )
        .await?;
    assert_ne!(demo.status_code(), 200, "{}", demo.body_str());

    // The plan's retention is a hard limit on the real route.
    let mut free = signup(&server).await?;
    conn.execute(
        "UPDATE account_subscriptions SET plan_type = 'free'
         WHERE account_id = (SELECT account_id FROM projects WHERE id = $1) AND ended_at IS NULL",
        &[&free.project_id.parse::<i32>().expect("numeric id")],
    )
    .await
    .expect("move to the free plan");
    free.log_in_again(&server).await?;
    let limits = free.get(&server, &free.path("")).await?;
    let oldest = limits["oldest_allowed_date"]
        .as_str()
        .expect("a retention limit");
    assert_eq!(limits["default_start_date"], json!(oldest));
    let too_old = jiff::civil::Date::strptime("%Y-%m-%d", oldest)
        .expect("date")
        .checked_sub(jiff::ToSpan::days(1))
        .expect("earlier")
        .to_string();
    let connection_id = free.connect(&server).await?;
    let refused = free.start(&server, &connection_id, &too_old).await?;
    assert_eq!(refused.status_code(), 400, "{}", refused.body_str());
    assert!(
        refused.body_str().contains("retention"),
        "{}",
        refused.body_str()
    );

    // Tracking older than the plan keeps: the defaults offer nothing, so they can't duplicate.
    let long_ago = now - 400 * 86_400;
    {
        let tracked =
            revscope::db::open_project_database_write(server.context.as_ref(), &free.project_id)
                .expect("project database");
        tracked
            .execute(
                "INSERT INTO paths (id, path, hostname) VALUES (1, '/', 'acme.test')",
                [],
            )
            .expect("path");
        tracked
            .execute(
                "INSERT INTO pageviews (timestamp_id, visitor_timestamp_id, path_id) VALUES (?1, ?1, 1)",
                [long_ago * 1_000_000_000],
            )
            .expect("pageview");
    }
    let limits = free.get(&server, &free.path("")).await?;
    let default_start = limits["default_start_date"].as_str().expect("start");
    let default_end = limits["default_end_date"].as_str().expect("end");
    assert!(default_end < default_start, "{limits}");
    let first_tracked = limits["first_tracked_date"].as_str().expect("tracked");
    let day_before_tracking = jiff::civil::Date::strptime("%Y-%m-%d", first_tracked)
        .expect("date")
        .checked_sub(jiff::ToSpan::days(1))
        .expect("earlier")
        .to_string();
    assert_eq!(default_end, day_before_tracking, "{limits}");
    let as_defaulted = free
        .post(
            &server,
            &free.path(""),
            json!({
                "connection_id": connection_id,
                "property_id": "123",
                "start_date": default_start,
                "end_date": default_end,
            }),
        )
        .await?;
    assert_eq!(
        as_defaulted.status_code(),
        400,
        "{}",
        as_defaulted.body_str()
    );
    assert!(
        as_defaulted.body_str().contains("must not be after"),
        "{}",
        as_defaulted.body_str()
    );
    // A range overlapping tracking needs acknowledging.
    let inside_tracking = free.start(&server, &connection_id, default_start).await?;
    assert_eq!(
        inside_tracking.status_code(),
        409,
        "{}",
        inside_tracking.body_str()
    );
    assert!(
        inside_tracking.body_str().contains("own tracking"),
        "{}",
        inside_tracking.body_str()
    );
    // Acknowledged, it goes ahead: duplicates are allowed when asked for.
    let acknowledged = free
        .post(
            &server,
            &free.path(""),
            json!({
                "connection_id": connection_id,
                "property_id": "123",
                "start_date": default_start,
                "end_date": default_start,
                "allow_duplicates": true,
            }),
        )
        .await?;
    assert_eq!(
        acknowledged.status_code(),
        200,
        "{}",
        acknowledged.body_str()
    );

    // Only admins.
    conn.execute(
        "UPDATE users SET is_admin = FALSE WHERE email = $1",
        &[&free.email],
    )
    .await
    .expect("demote");
    free.log_in_again(&server).await?;
    let member = server
        .get_query(&free.path(""), &[("Cookie", free.cookie.as_str())])
        .await?;
    assert_eq!(member.status_code(), 403, "{}", member.body_str());
    Ok(())
}
