use rusqlite::OptionalExtension;
use rusqlite::params;

use crate::db::ProjectConnection;

#[derive(Debug, Clone)]
pub(crate) struct PaymentFixture {
    pub(crate) subscription_id: Option<i64>,
    pub(crate) interval_unit: Option<String>,
    pub(crate) interval_count: Option<i64>,
    pub(crate) visitor_timestamp_id: Option<i64>,
    pub(crate) attributed_visitor_timestamp_id: Option<i64>,
    pub(crate) session_duration_seconds: Option<i64>,
    pub(crate) session_page_count: Option<i64>,
    pub(crate) provider_payment_id: String,
    pub(crate) provider_order_id: Option<String>,
    pub(crate) provider_invoice_id: Option<String>,
    pub(crate) provider_customer_id: Option<String>,
    pub(crate) customer_email: Option<String>,
    pub(crate) order_email: Option<String>,
    pub(crate) payment_type: String,
    pub(crate) status: String,
    pub(crate) amount: i64,
    pub(crate) mrr_amount: Option<i64>,
    pub(crate) currency: String,
    pub(crate) fee_amount: Option<i64>,
    pub(crate) net_amount: Option<i64>,
    pub(crate) paid_at: i64,
    pub(crate) created_at: i64,
    pub(crate) metadata: Option<String>,
}

impl PaymentFixture {
    pub(crate) fn new(provider_payment_id: impl Into<String>, amount: i64, paid_at: i64) -> Self {
        Self {
            subscription_id: None,
            interval_unit: None,
            interval_count: None,
            visitor_timestamp_id: None,
            attributed_visitor_timestamp_id: None,
            session_duration_seconds: None,
            session_page_count: None,
            provider_payment_id: provider_payment_id.into(),
            provider_order_id: None,
            provider_invoice_id: None,
            provider_customer_id: None,
            customer_email: None,
            order_email: None,
            payment_type: "single".to_string(),
            status: "ok".to_string(),
            amount,
            mrr_amount: None,
            currency: "USD".to_string(),
            fee_amount: None,
            net_amount: None,
            paid_at,
            created_at: paid_at,
            metadata: None,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SubscriptionFixture {
    pub(crate) provider_subscription_id: String,
    pub(crate) visitor_timestamp_id: Option<i64>,
    pub(crate) attributed_visitor_timestamp_id: Option<i64>,
    pub(crate) provider_customer_id: Option<String>,
    pub(crate) customer_email: Option<String>,
    pub(crate) order_email: Option<String>,
    pub(crate) interval_unit: String,
    pub(crate) interval_count: i64,
    /// Recurring price per billing interval, in thousandths.
    pub(crate) plan_amount: Option<i64>,
    pub(crate) current_period_start: Option<i64>,
    pub(crate) current_period_end: Option<i64>,
    pub(crate) started_at: Option<i64>,
    pub(crate) ended_at: Option<i64>,
    pub(crate) canceled_at: Option<i64>,
    pub(crate) cancel_at: Option<i64>,
    pub(crate) metadata: Option<String>,
}

impl SubscriptionFixture {
    pub(crate) fn new(provider_subscription_id: impl Into<String>) -> Self {
        Self {
            provider_subscription_id: provider_subscription_id.into(),
            visitor_timestamp_id: None,
            attributed_visitor_timestamp_id: None,
            provider_customer_id: None,
            customer_email: None,
            order_email: None,
            interval_unit: "month".to_string(),
            interval_count: 1,
            plan_amount: None,
            current_period_start: None,
            current_period_end: None,
            started_at: Some(0),
            ended_at: None,
            canceled_at: None,
            cancel_at: None,
            metadata: None,
        }
    }
}

pub(crate) fn insert_payment(conn: &ProjectConnection, payment: PaymentFixture) -> i64 {
    let attributed_visitor_timestamp_id = payment
        .attributed_visitor_timestamp_id
        .or(payment.visitor_timestamp_id);
    conn.execute(
        "INSERT INTO payments (
            provider, subscription_id, interval_unit, interval_count,
            visitor_timestamp_id, attributed_visitor_timestamp_id,
            session_duration_seconds, session_page_count,
            provider_payment_id, provider_order_id, provider_invoice_id,
            provider_customer_id, customer_email, order_email, payment_type, status,
            amount, mrr_amount, currency, fee_amount, net_amount,
            paid_at, created_at, metadata
         ) VALUES (
            'stripe', ?1, ?2, ?3,
            ?4, ?5,
            ?6, ?7,
            ?8, ?9, ?10,
            ?11, ?12, ?13, ?14, ?15,
            ?16, ?17, ?18, ?19, ?20,
            ?21, ?22, ?23
         )",
        params![
            payment.subscription_id,
            payment.interval_unit,
            payment.interval_count,
            payment.visitor_timestamp_id,
            attributed_visitor_timestamp_id,
            payment.session_duration_seconds,
            payment.session_page_count,
            payment.provider_payment_id,
            payment.provider_order_id,
            payment.provider_invoice_id,
            payment.provider_customer_id,
            payment.customer_email,
            payment.order_email,
            payment.payment_type,
            payment.status,
            payment.amount,
            payment.mrr_amount,
            payment.currency,
            payment.fee_amount,
            payment.net_amount,
            payment.paid_at,
            payment.created_at,
            payment.metadata,
        ],
    )
    .expect("insert payment");

    conn.query_row(
        "SELECT id FROM payments ORDER BY id DESC LIMIT 1",
        [],
        |row| row.get::<_, i64>(0),
    )
    .expect("payment id")
}

/// Inserts a subscription anchor row plus its opening history version, the way
/// a provider sync would.
pub(crate) fn insert_subscription(
    conn: &ProjectConnection,
    subscription: SubscriptionFixture,
) -> i64 {
    let mrr_amount = subscription.plan_amount.and_then(|amount| {
        crate::metrics::monthly_amount(
            amount,
            &subscription.interval_unit,
            subscription.interval_count,
        )
    });
    conn.execute(
        "INSERT INTO subscriptions (
            provider, provider_subscription_id, visitor_timestamp_id,
            attributed_visitor_timestamp_id, provider_customer_id,
            customer_email, order_email, interval_unit, interval_count,
            plan_amount, mrr_amount, current_period_start, current_period_end,
            started_at, ended_at, canceled_at, cancel_at, metadata
         ) VALUES (
            'stripe', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17
         )",
        params![
            subscription.provider_subscription_id,
            subscription.visitor_timestamp_id,
            subscription
                .attributed_visitor_timestamp_id
                .or(subscription.visitor_timestamp_id),
            subscription.provider_customer_id,
            subscription.customer_email,
            subscription.order_email,
            subscription.interval_unit,
            subscription.interval_count,
            subscription.plan_amount,
            mrr_amount,
            subscription.current_period_start,
            subscription.current_period_end,
            subscription.started_at,
            subscription.ended_at,
            subscription.canceled_at,
            subscription.cancel_at,
            subscription.metadata,
        ],
    )
    .expect("insert subscription");

    let subscription_id = conn
        .query_row(
            "SELECT id FROM subscriptions ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("subscription id");

    let mut version = SubscriptionVersionFixture::new(subscription.started_at.unwrap_or(0));
    version.interval_unit = subscription.interval_unit;
    version.interval_count = subscription.interval_count;
    version.plan_amount = subscription.plan_amount;
    version.mrr_amount = mrr_amount;
    version.canceled_at = subscription.canceled_at;
    version.cancel_at = subscription.cancel_at;
    version.ended_at = subscription.ended_at;
    insert_subscription_version(conn, subscription_id, version);
    subscription_id
}

/// One appended state of a subscription, as `subscription_history` records it.
#[derive(Debug, Clone)]
pub(crate) struct SubscriptionVersionFixture {
    pub(crate) effective_from: i64,
    pub(crate) change_kind: String,
    pub(crate) status: Option<String>,
    pub(crate) quantity: Option<i64>,
    pub(crate) interval_unit: String,
    pub(crate) interval_count: i64,
    pub(crate) plan_amount: Option<i64>,
    pub(crate) mrr_amount: Option<i64>,
    pub(crate) canceled_at: Option<i64>,
    pub(crate) cancel_at: Option<i64>,
    pub(crate) ended_at: Option<i64>,
}

impl SubscriptionVersionFixture {
    pub(crate) fn new(effective_from: i64) -> Self {
        Self {
            effective_from,
            change_kind: "created".to_string(),
            status: Some("active".to_string()),
            quantity: None,
            interval_unit: "month".to_string(),
            interval_count: 1,
            plan_amount: None,
            mrr_amount: None,
            canceled_at: None,
            cancel_at: None,
            ended_at: None,
        }
    }
}

/// Appends a history version, closing whichever version is currently open.
pub(crate) fn insert_subscription_version(
    conn: &ProjectConnection,
    subscription_id: i64,
    version: SubscriptionVersionFixture,
) {
    let open: Option<(i64, i64, Option<i64>)> = conn
        .query_row(
            "SELECT id, version, mrr_amount
             FROM subscription_history
             WHERE subscription_id = ?1 AND effective_to IS NULL",
            params![subscription_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .expect("open subscription version");

    let (version_number, prev_mrr_amount) = match open {
        Some((open_id, open_version, open_mrr_amount)) => {
            conn.execute(
                "UPDATE subscription_history SET effective_to = ?2 WHERE id = ?1",
                params![open_id, version.effective_from],
            )
            .expect("close subscription version");
            (open_version + 1, open_mrr_amount)
        }
        None => (1, None),
    };

    conn.execute(
        "INSERT INTO subscription_history (
            subscription_id, version, effective_from, observed_at, change_kind, status,
            quantity, currency, plan_amount, mrr_amount, prev_mrr_amount,
            interval_unit, interval_count, canceled_at, cancel_at, ended_at, fingerprint
         ) VALUES (
            ?1, ?2, ?3, ?3, ?4, ?5,
            ?6, 'USD', ?7, ?8, ?9,
            ?10, ?11, ?12, ?13, ?14, ?15
         )",
        params![
            subscription_id,
            version_number,
            version.effective_from,
            version.change_kind,
            version.status,
            version.quantity,
            version.plan_amount,
            version.mrr_amount,
            prev_mrr_amount,
            version.interval_unit,
            version.interval_count,
            version.canceled_at,
            version.cancel_at,
            version.ended_at,
            format!("fixture-{subscription_id}-{version_number}"),
        ],
    )
    .expect("insert subscription version");
}

pub(crate) fn visitor_timestamp_id_for_page_id(
    conn: &ProjectConnection,
    page_id: &[u8],
) -> Option<i64> {
    conn.query_row(
        "SELECT visitor_timestamp_id FROM client_page_ids_map WHERE page_id = ?1",
        params![page_id],
        |row| row.get::<_, i64>(0),
    )
    .optional()
    .expect("mapped landing timestamp query")
}

/// One request a [`StubHttpServer`] received.
#[cfg(feature = "billing")]
#[derive(Debug, Clone)]
pub(crate) struct StubRequest {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) query: String,
    pub(crate) form: Vec<(String, String)>,
}

#[cfg(feature = "billing")]
impl StubRequest {
    /// `"POST /v1/invoices"`, for asserting the order of calls.
    pub(crate) fn route(&self) -> String {
        format!("{} {}", self.method, self.path)
    }

    pub(crate) fn field(&self, key: &str) -> Option<&str> {
        self.form
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }
}

#[cfg(feature = "billing")]
type StubResponder = dyn Fn(&StubRequest) -> (u16, serde_json::Value) + Send + Sync;

/// A local HTTP server that answers each request with whatever `respond`
/// returns, and records the requests, for testing clients of external APIs
/// (Stripe and the like) without the network. Each response closes its
/// connection, so the request parser never has to handle pipelining.
#[cfg(feature = "billing")]
pub(crate) struct StubHttpServer {
    base_url: String,
    requests: std::sync::Arc<parking_lot::Mutex<Vec<StubRequest>>>,
}

#[cfg(feature = "billing")]
impl StubHttpServer {
    pub(crate) async fn start(
        respond: impl Fn(&StubRequest) -> (u16, serde_json::Value) + Send + Sync + 'static,
    ) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind stub server");
        let base_url = format!("http://{}", listener.local_addr().expect("stub address"));
        let requests = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
        let respond: std::sync::Arc<StubResponder> = std::sync::Arc::new(respond);
        let recorded = requests.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(serve_stub_request(
                    stream,
                    respond.clone(),
                    recorded.clone(),
                ));
            }
        });
        Self { base_url, requests }
    }

    pub(crate) fn base_url(&self) -> &str {
        &self.base_url
    }

    pub(crate) fn requests(&self) -> Vec<StubRequest> {
        self.requests.lock().clone()
    }

    pub(crate) fn routes(&self) -> Vec<String> {
        self.requests().iter().map(StubRequest::route).collect()
    }
}

#[cfg(feature = "billing")]
async fn serve_stub_request(
    mut stream: tokio::net::TcpStream,
    respond: std::sync::Arc<StubResponder>,
    recorded: std::sync::Arc<parking_lot::Mutex<Vec<StubRequest>>>,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
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
    let mut request_line = head.lines().next().unwrap_or_default().split(' ');
    let method = request_line.next().unwrap_or_default().to_string();
    let target = request_line.next().unwrap_or_default();
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let body = &buffer[head_end..head_end + content_length];
    let request = StubRequest {
        method,
        path: path.to_string(),
        query: query.to_string(),
        form: serde_urlencoded::from_bytes(body).unwrap_or_default(),
    };
    let (status, json) = respond(&request);
    recorded.lock().push(request);
    let body = json.to_string();
    let response = format!(
        "HTTP/1.1 {status} Stub\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

static MAIN_MIGRATIONS: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

/// A server context and a main Postgres connection for unit tests of code
/// that writes the main database. Like the e2e tests, `DATABASE_URL` comes
/// from the environment or `.e2e.env`, and is required: a skipped test would
/// read as a passing one.
pub(crate) async fn main_test_db(
    label: &str,
) -> (
    std::sync::Arc<crate::server_context::ServerContext>,
    crate::db::main::Connection,
) {
    crate::test_env::load_e2e_env();
    assert!(
        std::env::var_os("DATABASE_URL").is_some(),
        "DATABASE_URL must be set to run main-database tests (export it or add it to .e2e.env)"
    );
    let server_context =
        std::sync::Arc::new(crate::server_context::ServerContext::new_in_memory(label));
    MAIN_MIGRATIONS
        .get_or_init(|| async {
            crate::db::apply_main_migrations(server_context.clone())
                .await
                .expect("apply main migrations");
        })
        .await;
    let conn = crate::db::open_main_database_write(server_context.clone())
        .await
        .expect("open main database");
    (server_context, conn)
}

/// Inserts an account with a name no other test run uses, and returns its id.
pub(crate) async fn insert_test_account(conn: &crate::db::main::Connection, tag: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is after the unix epoch")
        .as_nanos();
    let name = format!("unit_{tag}_{nanos:x}");
    conn.query_one(
        "INSERT INTO accounts (name) VALUES ($1) RETURNING id::text",
        &[&name],
    )
    .await
    .expect("insert account")
    .get(0)
}

/// An in-memory project database with every migration applied.
pub(crate) fn open_migrated_db() -> ProjectConnection {
    let conn = crate::db::project::in_memory_test_connection();
    crate::db::migrations::apply_missing_project_migrations(&conn).expect("apply migrations");
    conn
}

/// A session's landing row plus its landing pageview. Columns left `None` are
/// stored as NULL; `device` defaults to `desktop`.
#[derive(Default)]
pub(crate) struct LandingFixture {
    pub(crate) visitor_timestamp_id: i64,
    pub(crate) path_id: i64,
    pub(crate) country: Option<&'static str>,
    pub(crate) region: Option<&'static str>,
    pub(crate) city: Option<&'static str>,
    pub(crate) browser_family: Option<&'static str>,
    pub(crate) os: Option<&'static str>,
    pub(crate) device: Option<&'static str>,
    pub(crate) referrer: Option<i64>,
    pub(crate) campaign: Option<i64>,
    pub(crate) referrer_type: Option<&'static str>,
}

pub(crate) fn insert_landing(conn: &ProjectConnection, landing: LandingFixture) {
    conn.execute(
        "INSERT INTO pageviews (timestamp_id, visitor_timestamp_id, path_id) VALUES (?1, ?1, ?2)",
        params![landing.visitor_timestamp_id, landing.path_id],
    )
    .expect("insert landing pageview");
    conn.execute(
        "INSERT INTO landings (
            visitor_timestamp_id, referrer, campaign, referrer_type, is_google, country,
            region, city, tracker_version, os, browser_family, device
         ) VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6, ?7, 1.0, ?8, ?9, ?10)",
        params![
            landing.visitor_timestamp_id,
            landing.referrer,
            landing.campaign,
            landing.referrer_type,
            landing.country,
            landing.region,
            landing.city,
            landing.os,
            landing.browser_family,
            landing.device.unwrap_or("desktop"),
        ],
    )
    .expect("insert landing");
}

/// Adds a later pageview of `path_id` to the session.
pub(crate) fn insert_pageview(
    conn: &ProjectConnection,
    timestamp_id: i64,
    visitor_timestamp_id: i64,
    path_id: i64,
) {
    conn.execute(
        "INSERT INTO pageviews (timestamp_id, visitor_timestamp_id, path_id) VALUES (?1, ?2, ?3)",
        params![timestamp_id, visitor_timestamp_id, path_id],
    )
    .expect("insert pageview");
}

static TEST_DIR_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A fresh directory under the system temp dir, removed when dropped.
pub(crate) struct TestDataDir {
    path: std::path::PathBuf,
}

impl TestDataDir {
    pub(crate) fn new(prefix: &str) -> Self {
        let unique_id = TEST_DIR_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "revscope-{prefix}-{}-{unique_id}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("create temporary test data directory");
        Self { path }
    }

    pub(crate) fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for TestDataDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
