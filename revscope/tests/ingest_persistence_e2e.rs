mod common;

use std::io;
use std::time::{SystemTime, UNIX_EPOCH};

use common::{
    ANDROID_UA, CHROME_UA, DEMO_PROJECT_ID, DEMO_PROJECT_ID_B, DEMO_PROJECT_NAME,
    DEMO_PROJECT_NAME_B, FIREFOX_UA, IngestJson, TestServer, TrackHit, batch_form,
    batch_form_with_send_time, count_rows, page_id, query_i64, query_optional_i64,
    query_optional_string, query_string,
};
use serde_json::json;

fn unix_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after unix epoch")
        .as_millis() as i64
}

#[tokio::test]
async fn single_pageview_persists_with_correct_shape() -> io::Result<()> {
    let mut server = TestServer::spawn("single-pageview").await?;
    let hit = TrackHit::pageview(DEMO_PROJECT_NAME, "/", &page_id(1));
    let response = server.post_ingest(&hit.to_form(), &[]).await?;
    assert_eq!(response.status_code(), 200);
    let raw_json: serde_json::Value = response.json();
    let session_hex = raw_json["visitor_timestamp_id"]
        .as_str()
        .expect("visitor_timestamp_id must be a hex string");
    assert_eq!(session_hex.len(), 16);
    assert!(session_hex.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert!(raw_json.get("page_id").is_none());
    let json: IngestJson = response.json();
    assert!(json.visitor_timestamp_id.is_some());
    assert!(json.storage_allowed);

    // Buffered, not yet flushed.
    {
        let conn = server.project_db(DEMO_PROJECT_ID);
        assert_eq!(count_rows(&conn, "pageviews"), 0);
        assert_eq!(count_rows(&conn, "landings"), 0);
    }

    server.flush().await;

    let conn = server.project_db(DEMO_PROJECT_ID);
    assert_eq!(count_rows(&conn, "pageviews"), 1);
    assert_eq!(count_rows(&conn, "landings"), 1);
    assert_eq!(count_rows(&conn, "user_sessions"), 1);
    assert_eq!(count_rows(&conn, "client_page_ids_map"), 1);
    assert_eq!(count_rows(&conn, "paths"), 1);

    let page_ts = query_i64(&conn, "SELECT timestamp_id FROM pageviews");
    let visitor_ts = query_i64(&conn, "SELECT visitor_timestamp_id FROM pageviews");
    assert_eq!(page_ts, visitor_ts);
    assert_eq!(page_ts, json.visitor_timestamp_id.unwrap());
    assert!(query_optional_i64(&conn, "SELECT previous_timestamp_id FROM pageviews").is_none());
    assert_eq!(query_string(&conn, "SELECT path FROM paths"), "/");
    assert_eq!(
        query_string(&conn, "SELECT hostname FROM paths"),
        "example.com"
    );
    assert_eq!(
        query_i64(&conn, "SELECT session_page_count FROM landings"),
        1
    );
    assert_eq!(
        query_string(&conn, "SELECT browser_family FROM landings"),
        "chrome"
    );
    assert_eq!(query_string(&conn, "SELECT os FROM landings"), "windows");
    assert_eq!(
        query_string(&conn, "SELECT device FROM landings"),
        "desktop"
    );
    assert_eq!(query_i64(&conn, "SELECT screen_width FROM landings"), 1920);
    assert_eq!(
        query_i64(&conn, "SELECT viewport_width FROM landings"),
        1280
    );
    assert_eq!(
        query_string(&conn, "SELECT timezone FROM landings"),
        "America/Denver"
    );
    assert_eq!(query_string(&conn, "SELECT language FROM landings"), "EN");
    assert_eq!(
        query_i64(
            &conn,
            "SELECT CAST(tracker_version AS INTEGER) FROM landings"
        ),
        1
    );
    Ok(())
}

#[tokio::test]
async fn context_hit_fills_the_landing_after_the_pageview_flushed() -> io::Result<()> {
    let mut server = TestServer::spawn("context-after-flush").await?;
    let landing = TrackHit::pageview_without_device_context(DEMO_PROJECT_NAME, "/", &page_id(1));
    let landing_json: IngestJson = server.post_ingest(&landing.to_form(), &[]).await?.json();
    assert!(
        landing_json.storage_allowed,
        "a consenting visitor's landing must allow storage"
    );
    server.flush().await;

    {
        let conn = server.project_db(DEMO_PROJECT_ID);
        assert!(query_optional_string(&conn, "SELECT timezone FROM landings").is_none());
        assert_eq!(query_i64(&conn, "SELECT screen_width FROM landings"), 0);
    }

    let context = TrackHit::context(DEMO_PROJECT_NAME, &page_id(1))
        .with_session(landing_json.visitor_timestamp_id.unwrap());
    let response = server.post_ingest(&context.to_form(), &[]).await?;
    assert_eq!(response.status_code(), 200);
    server.flush().await;

    let conn = server.project_db(DEMO_PROJECT_ID);
    assert_eq!(count_rows(&conn, "landings"), 1);
    assert_eq!(count_rows(&conn, "pageviews"), 1);
    assert_eq!(
        query_string(&conn, "SELECT timezone FROM landings"),
        "America/Denver"
    );
    assert_eq!(query_string(&conn, "SELECT language FROM landings"), "EN");
    assert_eq!(query_i64(&conn, "SELECT screen_width FROM landings"), 1920);
    assert_eq!(query_i64(&conn, "SELECT screen_height FROM landings"), 1080);
    assert_eq!(
        query_i64(&conn, "SELECT viewport_width FROM landings"),
        1280
    );
    assert_eq!(
        query_i64(&conn, "SELECT viewport_height FROM landings"),
        720
    );
    assert_eq!(
        query_string(&conn, "SELECT device FROM landings"),
        "desktop"
    );
    Ok(())
}

#[tokio::test]
async fn context_hit_batched_with_its_own_landing_still_applies() -> io::Result<()> {
    let mut server = TestServer::spawn("context-in-batch").await?;
    let landing = TrackHit::pageview_without_device_context(DEMO_PROJECT_NAME, "/", &page_id(1));
    let context = TrackHit::context(DEMO_PROJECT_NAME, &page_id(1)).with_touch(true);
    let body = batch_form(DEMO_PROJECT_NAME, &[landing, context]);
    let response = server.post_ingest(&body, &[]).await?;
    assert_eq!(response.status_code(), 200);
    server.flush().await;

    let conn = server.project_db(DEMO_PROJECT_ID);
    assert_eq!(
        query_string(&conn, "SELECT timezone FROM landings"),
        "America/Denver"
    );
    assert_eq!(query_i64(&conn, "SELECT screen_width FROM landings"), 1920);
    // Touch support only upgrades a desktop; it never overrules the user agent.
    assert_eq!(
        query_string(&conn, "SELECT device FROM landings"),
        "desktop_touch"
    );
    Ok(())
}

#[tokio::test]
async fn touch_context_does_not_downgrade_a_mobile_device() -> io::Result<()> {
    let mut server = TestServer::spawn("context-mobile-device").await?;
    let landing = TrackHit::pageview_without_device_context(DEMO_PROJECT_NAME, "/", &page_id(1));
    let landing_json: IngestJson = server
        .post_ingest(&landing.to_form(), &[("User-Agent", ANDROID_UA)])
        .await?
        .json();
    let context = TrackHit::context(DEMO_PROJECT_NAME, &page_id(1))
        .with_touch(true)
        .with_session(landing_json.visitor_timestamp_id.unwrap());
    server
        .post_ingest(&context.to_form(), &[("User-Agent", ANDROID_UA)])
        .await?;
    server.flush().await;

    let conn = server.project_db(DEMO_PROJECT_ID);
    assert_eq!(query_string(&conn, "SELECT device FROM landings"), "mobile");
    Ok(())
}

#[tokio::test]
async fn landing_language_falls_back_to_the_accept_language_header() -> io::Result<()> {
    let mut server = TestServer::spawn("accept-language-fallback").await?;
    let landing = TrackHit::pageview_without_device_context(DEMO_PROJECT_NAME, "/", &page_id(1));
    let response = server
        .post_ingest(
            &landing.to_form(),
            &[("Accept-Language", "fr-CH, fr;q=0.9, en;q=0.8")],
        )
        .await?;
    assert_eq!(response.status_code(), 200);
    server.flush().await;

    let conn = server.project_db(DEMO_PROJECT_ID);
    assert_eq!(query_string(&conn, "SELECT language FROM landings"), "FR");
    Ok(())
}

#[tokio::test]
async fn denied_consent_disallows_storage() -> io::Result<()> {
    let server = TestServer::spawn("context-consent-denied").await?;
    let landing = TrackHit::pageview_without_device_context(DEMO_PROJECT_NAME, "/", &page_id(1))
        .with_consent(false);
    let json: IngestJson = server.post_ingest(&landing.to_form(), &[]).await?.json();
    assert!(!json.storage_allowed);
    Ok(())
}

#[tokio::test]
async fn continuing_session_still_reports_storage_allowed() -> io::Result<()> {
    let server = TestServer::spawn("storage-allowed-refresh").await?;
    let landing = TrackHit::pageview_without_device_context(DEMO_PROJECT_NAME, "/", &page_id(1));
    let landing_json: IngestJson = server.post_ingest(&landing.to_form(), &[]).await?.json();
    assert!(landing_json.storage_allowed);
    server.flush().await;

    // An existing session id no longer suppresses the signal — the tracker
    // refreshes the cookie whenever storage is still allowed.
    let second = TrackHit::pageview_without_device_context(DEMO_PROJECT_NAME, "/next", &page_id(2))
        .with_session(landing_json.visitor_timestamp_id.unwrap());
    let second_json: IngestJson = server.post_ingest(&second.to_form(), &[]).await?.json();
    assert!(second_json.storage_allowed);
    Ok(())
}

#[tokio::test]
async fn unrecognized_browser_is_recorded_as_crawler_view() -> io::Result<()> {
    let mut server = TestServer::spawn("bot-landing").await?;
    let hit = TrackHit::pageview(DEMO_PROJECT_NAME, "/", &page_id(1));
    let response = server
        .post_ingest(
            &hit.to_form(),
            &[(
                "User-Agent",
                "Mozilla/5.0 (compatible; GPTBot/1.4; +https://openai.com/gptbot)",
            )],
        )
        .await?;
    assert_eq!(response.status_code(), 200);
    server.flush().await;

    let conn = server.project_db(DEMO_PROJECT_ID);
    assert_eq!(count_rows(&conn, "landings"), 0);
    assert_eq!(count_rows(&conn, "pageviews"), 0);
    assert_eq!(count_rows(&conn, "crawler_views_by_hour"), 1);
    assert_eq!(
        query_i64(&conn, "SELECT source FROM crawler_views_by_hour"),
        0
    );
    assert_eq!(
        query_i64(&conn, "SELECT verified FROM crawler_views_by_hour"),
        0
    );
    assert_eq!(
        query_i64(&conn, "SELECT estimated_count FROM crawler_views_by_hour"),
        1
    );
    assert_eq!(
        query_string(
            &conn,
            "SELECT u.useragent FROM crawler_views_by_hour c JOIN useragents u ON u.id = c.useragent_id"
        ),
        "mozilla/5.0 (compatible; gptbot/1.4; +https://openai.com/gptbot)"
    );
    assert_eq!(
        query_string(&conn, "SELECT operator FROM useragents"),
        "openai"
    );
    assert_eq!(
        query_string(&conn, "SELECT category FROM useragents"),
        "ai_crawler"
    );
    assert_eq!(
        query_string(&conn, "SELECT ip FROM useragents"),
        "127.0.0.1"
    );
    // Divert must not insert human `paths`; crawler hits use `crawler_paths`.
    assert_eq!(count_rows(&conn, "paths"), 0);
    assert_eq!(count_rows(&conn, "crawler_paths"), 1);
    assert_eq!(query_string(&conn, "SELECT path FROM crawler_paths"), "/");
    assert!(query_optional_i64(&conn, "SELECT path_id FROM crawler_views_by_hour").is_some());
    Ok(())
}

#[tokio::test]
async fn unrecognized_browser_without_known_token_has_null_operator() -> io::Result<()> {
    let mut server = TestServer::spawn("unknown-bot").await?;
    let hit = TrackHit::pageview(DEMO_PROJECT_NAME, "/", &page_id(1));
    server
        .post_ingest(
            &hit.to_form(),
            &[(
                "User-Agent",
                "Mozilla/5.0 (compatible; SomeCrawler/2.1; +http://crawler.example/bot)",
            )],
        )
        .await?;
    server.flush().await;

    let conn = server.project_db(DEMO_PROJECT_ID);
    assert_eq!(count_rows(&conn, "crawler_views_by_hour"), 1);
    assert!(query_optional_string(&conn, "SELECT operator FROM useragents").is_none());
    assert!(query_optional_string(&conn, "SELECT category FROM useragents").is_none());
    Ok(())
}

#[tokio::test]
async fn googlebot_is_verified_when_ip_matches_published_ranges() -> io::Result<()> {
    let mut server = TestServer::spawn("googlebot-verified").await?;
    let google_ua = [(
        "User-Agent",
        "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)",
    )];
    let verified = TrackHit::crawler(DEMO_PROJECT_NAME, "/");
    server
        .post_ingest(
            &verified.to_form(),
            &[google_ua[0], ("CF-Connecting-IP", "66.249.66.1")],
        )
        .await?;
    let spoofed = TrackHit::crawler(DEMO_PROJECT_NAME, "/");
    server.post_ingest(&spoofed.to_form(), &google_ua).await?;
    let mismatched = TrackHit::crawler(DEMO_PROJECT_NAME, "/");
    server
        .post_ingest(
            &mismatched.to_form(),
            &[
                (
                    "User-Agent",
                    "Mozilla/5.0 (compatible; GPTBot/1.4; +https://openai.com/gptbot)",
                ),
                ("CF-Connecting-IP", "66.249.66.1"),
            ],
        )
        .await?;
    server.flush().await;

    let conn = server.project_db(DEMO_PROJECT_ID);
    assert_eq!(count_rows(&conn, "crawler_views_by_hour"), 3);
    assert_eq!(
        query_i64(
            &conn,
            "SELECT COUNT(*) FROM crawler_views_by_hour cv
             JOIN useragents u ON u.id = cv.useragent_id
             WHERE u.operator = 'google' AND cv.verified = 1"
        ),
        1
    );
    assert_eq!(
        query_i64(
            &conn,
            "SELECT COUNT(*) FROM crawler_views_by_hour cv
             JOIN useragents u ON u.id = cv.useragent_id
             WHERE u.operator = 'google' AND cv.verified = 0"
        ),
        1
    );
    assert_eq!(
        query_i64(
            &conn,
            "SELECT COUNT(*) FROM crawler_views_by_hour cv
             JOIN useragents u ON u.id = cv.useragent_id
             WHERE u.operator = 'openai' AND cv.verified = 0"
        ),
        1
    );
    assert!(
        query_optional_string(
            &conn,
            "SELECT u.ip FROM crawler_views_by_hour cv
             JOIN useragents u ON u.id = cv.useragent_id
             WHERE u.operator = 'google' AND cv.verified = 1"
        )
        .is_none()
    );
    assert_eq!(
        query_string(
            &conn,
            "SELECT u.ip FROM crawler_views_by_hour cv
             JOIN useragents u ON u.id = cv.useragent_id
             WHERE u.operator = 'openai' AND cv.verified = 0"
        ),
        "66.249.66.1"
    );
    Ok(())
}

#[tokio::test]
async fn unverified_crawler_useragents_are_unique_per_ip() -> io::Result<()> {
    let mut server = TestServer::spawn("unverified-crawler-ip").await?;
    let ua = [(
        "User-Agent",
        "Mozilla/5.0 (compatible; GPTBot/1.4; +https://openai.com/gptbot)",
    )];
    let first = TrackHit::crawler(DEMO_PROJECT_NAME, "/");
    server
        .post_ingest(
            &first.to_form(),
            &[ua[0], ("CF-Connecting-IP", "203.0.113.10")],
        )
        .await?;
    let second = TrackHit::crawler(DEMO_PROJECT_NAME, "/");
    server
        .post_ingest(
            &second.to_form(),
            &[ua[0], ("CF-Connecting-IP", "2001:db8::1")],
        )
        .await?;
    let again = TrackHit::crawler(DEMO_PROJECT_NAME, "/");
    server
        .post_ingest(
            &again.to_form(),
            &[ua[0], ("CF-Connecting-IP", "203.0.113.10")],
        )
        .await?;
    server.flush().await;

    let conn = server.project_db(DEMO_PROJECT_ID);
    assert_eq!(count_rows(&conn, "useragents"), 2);
    assert_eq!(count_rows(&conn, "crawler_views_by_hour"), 2);
    assert_eq!(
        query_i64(
            &conn,
            "SELECT cv.estimated_count FROM crawler_views_by_hour cv
             JOIN useragents u ON u.id = cv.useragent_id
             WHERE u.ip = '203.0.113.10'"
        ),
        2
    );
    assert_eq!(
        query_string(
            &conn,
            "SELECT u.ip FROM crawler_views_by_hour cv
             JOIN useragents u ON u.id = cv.useragent_id
             WHERE u.ip = '2001:db8::1'"
        ),
        "2001:db8::1"
    );
    Ok(())
}

#[tokio::test]
async fn crawler_hits_aggregate_by_hour_and_link_known_paths() -> io::Result<()> {
    let mut server = TestServer::spawn("crawler-hits").await?;
    let landing = TrackHit::pageview(DEMO_PROJECT_NAME, "/pricing", &page_id(1));
    server.post_ingest(&landing.to_form(), &[]).await?;
    server.flush().await;

    let ua = [(
        "User-Agent",
        "Mozilla/5.0 (compatible; GPTBot/1.4; +https://openai.com/gptbot)",
    )];
    let known = TrackHit::crawler(DEMO_PROJECT_NAME, "/pricing");
    let unknown = TrackHit::crawler(DEMO_PROJECT_NAME, "/robots.txt");
    server.post_ingest(&known.to_form(), &ua).await?;
    server.post_ingest(&known.to_form(), &ua).await?;
    server.post_ingest(&unknown.to_form(), &ua).await?;
    server.flush().await;

    let conn = server.project_db(DEMO_PROJECT_ID);
    assert_eq!(count_rows(&conn, "pageviews"), 1);
    assert_eq!(count_rows(&conn, "paths"), 1);
    assert_eq!(count_rows(&conn, "crawler_paths"), 2);
    assert_eq!(count_rows(&conn, "crawler_views_by_hour"), 2);
    assert_eq!(
        query_i64(
            &conn,
            "SELECT cv.estimated_count FROM crawler_views_by_hour cv
             JOIN crawler_paths p ON p.id = cv.path_id
             WHERE p.path = '/pricing'"
        ),
        2
    );
    assert_eq!(
        query_i64(
            &conn,
            "SELECT cv.estimated_count FROM crawler_views_by_hour cv
             JOIN crawler_paths p ON p.id = cv.path_id
             WHERE p.path = '/robots.txt'"
        ),
        1
    );
    Ok(())
}

#[tokio::test]
async fn returning_visitor_continues_session() -> io::Result<()> {
    let mut server = TestServer::spawn("returning-visitor").await?;
    let first = TrackHit::pageview(DEMO_PROJECT_NAME, "/", &page_id(1));
    let first_json: IngestJson = server.post_ingest(&first.to_form(), &[]).await?.json();
    server.flush().await;

    let second = TrackHit::pageview(DEMO_PROJECT_NAME, "/pricing", &page_id(2))
        .with_session(first_json.visitor_timestamp_id.unwrap());
    let second_json: IngestJson = server.post_ingest(&second.to_form(), &[]).await?.json();
    server.flush().await;

    assert_eq!(
        second_json.visitor_timestamp_id,
        first_json.visitor_timestamp_id
    );

    let conn = server.project_db(DEMO_PROJECT_ID);
    assert_eq!(count_rows(&conn, "pageviews"), 2);
    assert_eq!(count_rows(&conn, "landings"), 1);
    assert_eq!(
        query_i64(&conn, "SELECT session_page_count FROM landings"),
        2
    );
    let first_page = query_i64(
        &conn,
        "SELECT timestamp_id FROM pageviews ORDER BY timestamp_id ASC LIMIT 1",
    );
    let previous = query_optional_i64(
        &conn,
        "SELECT previous_timestamp_id FROM pageviews ORDER BY timestamp_id DESC LIMIT 1",
    );
    assert_eq!(previous, Some(first_page));
    Ok(())
}

#[tokio::test]
async fn distinct_visitors_get_distinct_sessions() -> io::Result<()> {
    let mut server = TestServer::spawn("distinct-visitors").await?;
    let a = TrackHit::pageview(DEMO_PROJECT_NAME, "/", &page_id(1));
    let b = TrackHit::pageview(DEMO_PROJECT_NAME, "/", &page_id(2));
    let a_json: IngestJson = server
        .post_ingest(
            &a.to_form(),
            &[
                ("X-Forwarded-For", "203.0.113.10"),
                ("User-Agent", CHROME_UA),
            ],
        )
        .await?
        .json();
    let b_json: IngestJson = server
        .post_ingest(
            &b.to_form(),
            &[
                ("X-Forwarded-For", "203.0.113.20"),
                ("User-Agent", FIREFOX_UA),
            ],
        )
        .await?
        .json();
    server.flush().await;

    assert_ne!(a_json.visitor_timestamp_id, b_json.visitor_timestamp_id);
    let conn = server.project_db(DEMO_PROJECT_ID);
    assert_eq!(count_rows(&conn, "pageviews"), 2);
    assert_eq!(count_rows(&conn, "landings"), 2);
    assert_eq!(count_rows(&conn, "user_sessions"), 2);
    Ok(())
}

#[tokio::test]
async fn session_cookie_lifecycle_on_gif_route() -> io::Result<()> {
    let server = TestServer::spawn("session-cookie").await?;
    let hit = TrackHit::pageview(DEMO_PROJECT_NAME, "/", &page_id(1)).with_consent(true);
    let response = server.get_gif(&hit.to_form(), &[]).await?;
    assert_eq!(response.status_code(), 200);
    let session_cookie_prefix = format!("rs{DEMO_PROJECT_NAME}=");
    assert!(
        response
            .set_cookies
            .iter()
            .any(|cookie| cookie.starts_with(&session_cookie_prefix)),
        "expected {session_cookie_prefix} Set-Cookie, got {:?}",
        response.set_cookies
    );
    let cookie_value = response
        .set_cookies
        .iter()
        .find_map(|cookie| cookie.strip_prefix(&session_cookie_prefix))
        .and_then(|rest| rest.split(';').next())
        .expect("cookie value");
    let visitor_id =
        revscope::util::parse_visitor_timestamp_id_hex(cookie_value).expect("hex visitor id");
    server.flush().await;

    let returning = TrackHit::pageview(DEMO_PROJECT_NAME, "/about", &page_id(2))
        .with_session(visitor_id)
        .with_consent(true);
    let returning_json: IngestJson = server.post_ingest(&returning.to_form(), &[]).await?.json();
    assert_eq!(returning_json.visitor_timestamp_id, Some(visitor_id));
    assert!(returning_json.storage_allowed);

    let cleared = TrackHit::pageview(DEMO_PROJECT_NAME, "/bye", &page_id(3)).with_consent(false);
    let cleared_response = server.get_gif(&cleared.to_form(), &[]).await?;
    assert!(
        cleared_response.set_cookies.iter().any(|cookie| {
            cookie.contains(&session_cookie_prefix) && cookie.contains("Max-Age=0")
        }),
        "expected cleared cookie, got {:?}",
        cleared_response.set_cookies
    );

    let no_persist: IngestJson = server
        .post_ingest(
            &TrackHit::pageview(DEMO_PROJECT_NAME, "/x", &page_id(4))
                .with_consent(false)
                .to_form(),
            &[],
        )
        .await?
        .json();
    assert!(!no_persist.storage_allowed);
    Ok(())
}

#[tokio::test]
async fn landing_records_nullable_consent_signal() -> io::Result<()> {
    let mut server = TestServer::spawn("landing-consent").await?;
    let granted = TrackHit::pageview(DEMO_PROJECT_NAME, "/granted", &page_id(1));
    let denied = TrackHit::pageview(DEMO_PROJECT_NAME, "/denied", &page_id(2)).with_consent(false);
    let mut unknown = TrackHit::pageview(DEMO_PROJECT_NAME, "/unknown", &page_id(3));
    unknown.consent = None;

    server
        .post_ingest(&granted.to_form(), &[("X-Forwarded-For", "198.51.100.1")])
        .await?;
    server
        .post_ingest(
            &denied.to_form(),
            &[
                ("X-Forwarded-For", "198.51.100.2"),
                ("User-Agent", FIREFOX_UA),
            ],
        )
        .await?;
    server
        .post_ingest(&unknown.to_form(), &[("X-Forwarded-For", "198.51.100.3")])
        .await?;
    server.flush().await;

    let conn = server.project_db(DEMO_PROJECT_ID);
    let consent_for_path = |path: &str| {
        query_optional_i64(
            &conn,
            &format!(
                "SELECT landing.consent
                 FROM landings landing
                 JOIN pageviews page ON page.timestamp_id = landing.visitor_timestamp_id
                 JOIN paths path ON path.id = page.path_id
                 WHERE path.path = '{path}'"
            ),
        )
    };
    assert_eq!(consent_for_path("/granted"), Some(1));
    assert_eq!(consent_for_path("/denied"), Some(0));
    assert_eq!(consent_for_path("/unknown"), None);
    Ok(())
}

#[tokio::test]
async fn client_page_id_targets_existing_pageview_after_flush() -> io::Result<()> {
    let mut server = TestServer::spawn("pg-target").await?;
    let pageview = TrackHit::pageview(DEMO_PROJECT_NAME, "/", &page_id(1));
    let page_json: IngestJson = server.post_ingest(&pageview.to_form(), &[]).await?.json();
    server.flush().await;

    let engagement = TrackHit::engagement(DEMO_PROJECT_NAME, &page_id(1), 12, 40)
        .with_session(page_json.visitor_timestamp_id.unwrap());
    let eng_json: IngestJson = server.post_ingest(&engagement.to_form(), &[]).await?.json();
    assert_eq!(
        eng_json.visitor_timestamp_id,
        page_json.visitor_timestamp_id
    );
    server.flush().await;

    let conn = server.project_db(DEMO_PROJECT_ID);
    assert_eq!(count_rows(&conn, "pageviews"), 1);
    assert_eq!(
        query_i64(&conn, "SELECT duration_seconds FROM pageviews"),
        12
    );
    assert_eq!(
        query_i64(&conn, "SELECT scrolled_percent FROM pageviews"),
        40
    );
    Ok(())
}

#[tokio::test]
async fn external_google_referrer_sets_landing_attribution() -> io::Result<()> {
    let mut server = TestServer::spawn("google-referrer").await?;
    let hit = TrackHit::pageview(DEMO_PROJECT_NAME, "/", &page_id(1))
        .with_referrer("https://www.google.com/search?q=revscope");
    server.post_ingest(&hit.to_form(), &[]).await?;
    server.flush().await;

    let conn = server.project_db(DEMO_PROJECT_ID);
    assert_eq!(count_rows(&conn, "referrers"), 1);
    assert_eq!(query_i64(&conn, "SELECT is_google FROM landings"), 1);
    assert_eq!(
        query_string(&conn, "SELECT referrer_type FROM landings"),
        "search"
    );
    assert!(query_optional_i64(&conn, "SELECT referrer FROM landings").is_some());
    Ok(())
}

#[tokio::test]
async fn campaign_params_create_and_reuse_campaign_row() -> io::Result<()> {
    let mut server = TestServer::spawn("campaign-params").await?;
    let meta = json!({
        "source": "google",
        "medium": "cpc",
        "campaign": "brand",
        "gclid": "abc123"
    });
    // Google Ads often arrive with a google.com referrer plus a click id; is_google
    // must stay false so the landings CHECK (is_google => referrer_type=search) holds.
    let first = TrackHit::pageview(DEMO_PROJECT_NAME, "/", &page_id(1))
        .with_referrer("https://www.google.com/")
        .with_metadata(meta.clone());
    server
        .post_ingest(&first.to_form(), &[("X-Forwarded-For", "203.0.113.30")])
        .await?;
    server.flush().await;

    {
        let conn = server.project_db(DEMO_PROJECT_ID);
        assert_eq!(count_rows(&conn, "campaigns"), 1);
        assert_eq!(
            query_string(&conn, "SELECT source FROM campaigns"),
            "google"
        );
        assert_eq!(query_string(&conn, "SELECT medium FROM campaigns"), "cpc");
        assert_eq!(
            query_string(&conn, "SELECT campaign FROM campaigns"),
            "brand"
        );
        assert_eq!(
            query_optional_string(&conn, "SELECT network FROM campaigns").as_deref(),
            Some("google")
        );
        assert_eq!(
            query_string(&conn, "SELECT click_id FROM landings"),
            "abc123"
        );
        assert_eq!(
            query_string(&conn, "SELECT referrer_type FROM landings"),
            "campaign"
        );
        assert_eq!(query_i64(&conn, "SELECT is_google FROM landings"), 0);
    }

    let mixed_case = json!({
        "source": " Google ",
        "medium": " CPC ",
        "campaign": " Brand ",
        "gclid": "xyz789"
    });
    let second = TrackHit::pageview(DEMO_PROJECT_NAME, "/", &page_id(2)).with_metadata(mixed_case);
    server
        .post_ingest(
            &second.to_form(),
            &[
                ("X-Forwarded-For", "203.0.113.31"),
                ("User-Agent", FIREFOX_UA),
            ],
        )
        .await?;
    server.flush().await;

    let conn = server.project_db(DEMO_PROJECT_ID);
    assert_eq!(count_rows(&conn, "campaigns"), 1);
    assert_eq!(count_rows(&conn, "landings"), 2);
    Ok(())
}

#[tokio::test]
async fn self_referrer_is_ignored() -> io::Result<()> {
    let mut server = TestServer::spawn("self-referrer").await?;
    let hit = TrackHit::pageview(DEMO_PROJECT_NAME, "/pricing", &page_id(1))
        .with_referrer("https://example.com/home");
    server.post_ingest(&hit.to_form(), &[]).await?;
    server.flush().await;

    let conn = server.project_db(DEMO_PROJECT_ID);
    assert_eq!(count_rows(&conn, "referrers"), 0);
    assert!(query_optional_i64(&conn, "SELECT referrer FROM landings").is_none());
    assert_eq!(query_i64(&conn, "SELECT is_google FROM landings"), 0);
    Ok(())
}

#[tokio::test]
async fn custom_event_with_metadata_persists() -> io::Result<()> {
    let mut server = TestServer::spawn("custom-event").await?;
    let page = TrackHit::pageview(DEMO_PROJECT_NAME, "/", &page_id(1));
    let event = TrackHit::event(DEMO_PROJECT_NAME, "signup_click", &page_id(1))
        .with_event_metadata(json!({"plan": "pro", "count": 2}));
    let body = batch_form(DEMO_PROJECT_NAME, &[page, event]);
    let response = server.post_ingest(&body, &[]).await?;
    assert_eq!(response.status_code(), 200);
    server.flush().await;

    let conn = server.project_db(DEMO_PROJECT_ID);
    assert_eq!(count_rows(&conn, "events"), 1);
    assert_eq!(count_rows(&conn, "event_names"), 1);
    assert_eq!(
        query_string(&conn, "SELECT name FROM event_names"),
        "signup_click"
    );
    let page_ts = query_i64(&conn, "SELECT timestamp_id FROM pageviews");
    assert_eq!(
        query_i64(&conn, "SELECT page_timestamp_id FROM events"),
        page_ts
    );
    let metadata = query_string(&conn, "SELECT json(metadata) FROM events");
    let parsed: serde_json::Value = serde_json::from_str(&metadata).expect("event metadata json");
    assert_eq!(parsed["plan"], "pro");
    assert_eq!(parsed["count"], 2);
    Ok(())
}

#[tokio::test]
async fn event_name_is_deduplicated() -> io::Result<()> {
    let mut server = TestServer::spawn("event-name-dedup").await?;
    let page = TrackHit::pageview(DEMO_PROJECT_NAME, "/", &page_id(1));
    let e1 = TrackHit::event(DEMO_PROJECT_NAME, "click", &page_id(1));
    let e2 = TrackHit::event(DEMO_PROJECT_NAME, "click", &page_id(1));
    server
        .post_ingest(&batch_form(DEMO_PROJECT_NAME, &[page, e1, e2]), &[])
        .await?;
    server.flush().await;

    let conn = server.project_db(DEMO_PROJECT_ID);
    assert_eq!(count_rows(&conn, "event_names"), 1);
    assert_eq!(count_rows(&conn, "events"), 2);
    Ok(())
}

#[tokio::test]
async fn event_against_flushed_pageview_resolves_page() -> io::Result<()> {
    let mut server = TestServer::spawn("event-flushed-page").await?;
    let page = TrackHit::pageview(DEMO_PROJECT_NAME, "/", &page_id(1));
    let page_json: IngestJson = server.post_ingest(&page.to_form(), &[]).await?.json();
    server.flush().await;
    let page_ts = query_i64(
        &server.project_db(DEMO_PROJECT_ID),
        "SELECT timestamp_id FROM pageviews",
    );

    let event = TrackHit::event(DEMO_PROJECT_NAME, "purchase", &page_id(1))
        .with_session(page_json.visitor_timestamp_id.unwrap());
    server.post_ingest(&event.to_form(), &[]).await?;
    server.flush().await;

    let conn = server.project_db(DEMO_PROJECT_ID);
    assert_eq!(count_rows(&conn, "events"), 1);
    assert_eq!(
        query_i64(&conn, "SELECT page_timestamp_id FROM events"),
        page_ts
    );
    Ok(())
}

#[tokio::test]
async fn engagement_folded_into_same_batch_pageview() -> io::Result<()> {
    let mut server = TestServer::spawn("engagement-fold").await?;
    let page = TrackHit::pageview(DEMO_PROJECT_NAME, "/", &page_id(1));
    let engagement = TrackHit::engagement(DEMO_PROJECT_NAME, &page_id(1), 15, 55);
    server
        .post_ingest(&batch_form(DEMO_PROJECT_NAME, &[page, engagement]), &[])
        .await?;
    server.flush().await;

    let conn = server.project_db(DEMO_PROJECT_ID);
    assert_eq!(
        query_i64(&conn, "SELECT duration_seconds FROM pageviews"),
        15
    );
    assert_eq!(
        query_i64(&conn, "SELECT scrolled_percent FROM pageviews"),
        55
    );
    assert_eq!(
        query_i64(&conn, "SELECT session_duration_seconds FROM landings"),
        15
    );
    Ok(())
}

#[tokio::test]
async fn engagement_updates_already_flushed_pageview() -> io::Result<()> {
    let mut server = TestServer::spawn("engagement-update").await?;
    let page = TrackHit::pageview(DEMO_PROJECT_NAME, "/", &page_id(1));
    let page_json: IngestJson = server.post_ingest(&page.to_form(), &[]).await?.json();
    server.flush().await;

    let engagement = TrackHit::engagement(DEMO_PROJECT_NAME, &page_id(1), 8, 30)
        .with_session(page_json.visitor_timestamp_id.unwrap());
    server.post_ingest(&engagement.to_form(), &[]).await?;
    server.flush().await;

    let conn = server.project_db(DEMO_PROJECT_ID);
    assert_eq!(
        query_i64(&conn, "SELECT duration_seconds FROM pageviews"),
        8
    );
    assert_eq!(
        query_i64(&conn, "SELECT scrolled_percent FROM pageviews"),
        30
    );
    assert_eq!(
        query_i64(&conn, "SELECT session_duration_seconds FROM landings"),
        8
    );
    Ok(())
}

#[tokio::test]
async fn multiple_engagements_accumulate() -> io::Result<()> {
    let mut server = TestServer::spawn("engagement-accumulate").await?;
    let page = TrackHit::pageview(DEMO_PROJECT_NAME, "/", &page_id(1));
    let e1 = TrackHit::engagement(DEMO_PROJECT_NAME, &page_id(1), 5, 20);
    let e2 = TrackHit::engagement(DEMO_PROJECT_NAME, &page_id(1), 7, 60);
    let e3 = TrackHit::engagement(DEMO_PROJECT_NAME, &page_id(1), 3, 40);
    server
        .post_ingest(&batch_form(DEMO_PROJECT_NAME, &[page, e1, e2, e3]), &[])
        .await?;
    server.flush().await;

    let conn = server.project_db(DEMO_PROJECT_ID);
    assert_eq!(
        query_i64(&conn, "SELECT duration_seconds FROM pageviews"),
        15
    );
    assert_eq!(
        query_i64(&conn, "SELECT scrolled_percent FROM pageviews"),
        60
    );
    Ok(())
}

#[tokio::test]
async fn mixed_batch_envelope_persists() -> io::Result<()> {
    let mut server = TestServer::spawn("mixed-batch").await?;
    let page_a = TrackHit::pageview(DEMO_PROJECT_NAME, "/", &page_id(1));
    let page_b = TrackHit::pageview(DEMO_PROJECT_NAME, "/docs", &page_id(2));
    let event = TrackHit::event(DEMO_PROJECT_NAME, "cta", &page_id(1));
    let engagement = TrackHit::engagement(DEMO_PROJECT_NAME, &page_id(2), 4, 10);
    let response = server
        .post_ingest(
            &batch_form(DEMO_PROJECT_NAME, &[page_a, page_b, event, engagement]),
            &[],
        )
        .await?;
    assert_eq!(response.status_code(), 200);
    let json: IngestJson = response.json();
    assert!(json.visitor_timestamp_id.is_some());
    server.flush().await;

    let conn = server.project_db(DEMO_PROJECT_ID);
    assert_eq!(count_rows(&conn, "pageviews"), 2);
    assert_eq!(count_rows(&conn, "events"), 1);
    assert_eq!(
        query_i64(
            &conn,
            "SELECT duration_seconds FROM pageviews ORDER BY timestamp_id DESC LIMIT 1"
        ),
        4
    );
    Ok(())
}

#[tokio::test]
async fn envelope_send_time_backdates_queued_hits() -> io::Result<()> {
    let mut server = TestServer::spawn("send-time").await?;
    let send_time = unix_millis();
    // The tracker held this pageview for 20s before flushing its queue, and the
    // event fired 5s before that flush.
    let mut page = TrackHit::pageview(DEMO_PROJECT_NAME, "/queued", &page_id(1));
    page.time = Some(send_time - 20_000);
    let mut event = TrackHit::event(DEMO_PROJECT_NAME, "cta", &page_id(1));
    event.time = Some(send_time - 5_000);

    let response = server
        .post_ingest(&batch_form_with_send_time(&[page, event], send_time), &[])
        .await?;
    assert_eq!(response.status_code(), 200);
    server.flush().await;

    let conn = server.project_db(DEMO_PROJECT_ID);
    let page_age_millis =
        unix_millis() - query_i64(&conn, "SELECT timestamp_id FROM pageviews") / 1_000_000;
    let event_age_millis =
        unix_millis() - query_i64(&conn, "SELECT timestamp_id FROM events") / 1_000_000;
    assert!(
        (20_000..25_000).contains(&page_age_millis),
        "pageview id should sit ~20s in the past, got {page_age_millis}ms"
    );
    assert!(
        (5_000..10_000).contains(&event_age_millis),
        "event id should sit ~5s in the past, got {event_age_millis}ms"
    );
    Ok(())
}

#[tokio::test]
async fn backdated_pageview_keeps_its_events_after_it() -> io::Result<()> {
    let mut server = TestServer::spawn("send-time-floor").await?;
    // Recent traffic pushes the pageview floor up to "now", so the aged pageview
    // below cannot be backdated while its event still can.
    server
        .post_ingest(
            &TrackHit::pageview(DEMO_PROJECT_NAME, "/recent", &page_id(1)).to_form(),
            &[],
        )
        .await?;
    server.flush().await;

    let send_time = unix_millis();
    let mut page = TrackHit::pageview(DEMO_PROJECT_NAME, "/queued", &page_id(2));
    page.time = Some(send_time - 30_000);
    let mut event = TrackHit::event(DEMO_PROJECT_NAME, "cta", &page_id(2));
    event.time = Some(send_time - 5_000);

    let response = server
        .post_ingest(&batch_form_with_send_time(&[page, event], send_time), &[])
        .await?;
    assert_eq!(response.status_code(), 200);
    server.flush().await;

    let conn = server.project_db(DEMO_PROJECT_ID);
    let page_ts = query_i64(
        &conn,
        "SELECT timestamp_id FROM pageviews ORDER BY timestamp_id DESC LIMIT 1",
    );
    let event_ts = query_i64(&conn, "SELECT timestamp_id FROM events");
    assert_eq!(
        query_i64(&conn, "SELECT page_timestamp_id FROM events"),
        page_ts
    );
    assert!(
        event_ts > page_ts,
        "event {event_ts} should not predate its pageview {page_ts}"
    );
    Ok(())
}

#[tokio::test]
async fn batch_over_limit_is_rejected_with_nothing_persisted() -> io::Result<()> {
    let mut server = TestServer::spawn("batch-limit").await?;
    let mut items = Vec::with_capacity(21);
    items.push(TrackHit::pageview(DEMO_PROJECT_NAME, "/", &page_id(1)));
    for i in 0..20 {
        items.push(
            TrackHit::event(DEMO_PROJECT_NAME, "click", &page_id(1))
                .with_event_metadata(json!({"i": i})),
        );
    }
    let response = server
        .post_ingest(&batch_form(DEMO_PROJECT_NAME, &items), &[])
        .await?;
    assert_eq!(response.status_code(), 400);
    server.flush().await;

    let conn = server.project_db(DEMO_PROJECT_ID);
    assert_eq!(count_rows(&conn, "pageviews"), 0);
    assert_eq!(count_rows(&conn, "events"), 0);
    Ok(())
}

#[tokio::test]
async fn pageview_metadata_rows_upsert() -> io::Result<()> {
    let mut server = TestServer::spawn("metadata-rows").await?;
    let first = TrackHit::pageview(DEMO_PROJECT_NAME, "/", &page_id(1)).with_metadata(json!({
        "plan": "free",
        "count": 3,
        "active": true
    }));
    let first_json: IngestJson = server.post_ingest(&first.to_form(), &[]).await?.json();
    server.flush().await;

    {
        let conn = server.project_db(DEMO_PROJECT_ID);
        assert_eq!(count_rows(&conn, "metadata"), 3);
        assert_eq!(
            query_string(&conn, "SELECT value FROM metadata WHERE key = 'plan'"),
            "free"
        );
        assert_eq!(
            query_string(&conn, "SELECT value FROM metadata WHERE key = 'count'"),
            "3"
        );
        assert_eq!(
            query_string(&conn, "SELECT value FROM metadata WHERE key = 'active'"),
            "true"
        );
        assert_eq!(
            query_i64(
                &conn,
                "SELECT page_timestamp_id FROM metadata WHERE key = 'plan'"
            ),
            query_i64(&conn, "SELECT timestamp_id FROM pageviews")
        );
    }

    let second = TrackHit::pageview(DEMO_PROJECT_NAME, "/account", &page_id(2))
        .with_session(first_json.visitor_timestamp_id.unwrap())
        .with_metadata(json!({"plan": "pro"}));
    server.post_ingest(&second.to_form(), &[]).await?;
    server.flush().await;

    let conn = server.project_db(DEMO_PROJECT_ID);
    assert_eq!(
        query_string(&conn, "SELECT value FROM metadata WHERE key = 'plan'"),
        "pro"
    );
    assert_eq!(count_rows(&conn, "metadata"), 3);
    Ok(())
}

#[tokio::test]
async fn empty_string_metadata_value_deletes_key() -> io::Result<()> {
    let mut server = TestServer::spawn("metadata-delete").await?;
    let first = TrackHit::pageview(DEMO_PROJECT_NAME, "/", &page_id(1)).with_metadata(json!({
        "plan": "free",
        "region": "us"
    }));
    let first_json: IngestJson = server.post_ingest(&first.to_form(), &[]).await?.json();
    server.flush().await;

    {
        let conn = server.project_db(DEMO_PROJECT_ID);
        assert_eq!(count_rows(&conn, "metadata"), 2);
    }

    let clear = TrackHit::metadata(
        DEMO_PROJECT_NAME,
        &page_id(1),
        json!({"plan": "", "region": "eu"}),
    )
    .with_session(first_json.visitor_timestamp_id.unwrap());
    server.post_ingest(&clear.to_form(), &[]).await?;
    server.flush().await;

    let conn = server.project_db(DEMO_PROJECT_ID);
    assert_eq!(count_rows(&conn, "metadata"), 1);
    assert_eq!(
        query_string(&conn, "SELECT value FROM metadata WHERE key = 'region'"),
        "eu"
    );
    assert_eq!(
        count_rows(&conn, "metadata WHERE key = 'plan'"),
        0,
        "empty string should delete the plan key"
    );
    Ok(())
}

#[tokio::test]
async fn metadata_hits_batch_and_apply_in_enqueue_order_on_flush() -> io::Result<()> {
    let mut server = TestServer::spawn("metadata-hits").await?;
    let first = TrackHit::pageview(DEMO_PROJECT_NAME, "/", &page_id(1))
        .with_metadata(json!({"plan": "free", "stable": "same"}));
    let update = TrackHit::metadata(
        DEMO_PROJECT_NAME,
        &page_id(1),
        json!({"plan": "pro", "region": "us"}),
    );
    let second = TrackHit::pageview(DEMO_PROJECT_NAME, "/account", &page_id(2))
        .with_metadata(json!({"plan": "team"}));
    let response = server
        .post_ingest(
            &batch_form(DEMO_PROJECT_NAME, &[first, update, second]),
            &[],
        )
        .await?;
    assert_eq!(response.status_code(), 200);

    {
        let conn = server.project_db(DEMO_PROJECT_ID);
        assert_eq!(count_rows(&conn, "metadata"), 0);
    }

    server.flush().await;
    {
        let conn = server.project_db(DEMO_PROJECT_ID);
        assert_eq!(
            query_string(&conn, "SELECT value FROM metadata WHERE key = 'plan'"),
            "team"
        );
        assert_eq!(
            query_string(&conn, "SELECT value FROM metadata WHERE key = 'stable'"),
            "same"
        );
        assert_eq!(
            query_string(&conn, "SELECT value FROM metadata WHERE key = 'region'"),
            "us"
        );
    }

    let deferred = TrackHit::metadata(DEMO_PROJECT_NAME, &page_id(2), json!({"region": "eu"}));
    let response = server.post_ingest(&deferred.to_form(), &[]).await?;
    assert_eq!(response.status_code(), 200);
    {
        let conn = server.project_db(DEMO_PROJECT_ID);
        assert_eq!(
            query_string(&conn, "SELECT value FROM metadata WHERE key = 'region'"),
            "us"
        );
    }

    server.flush().await;
    let conn = server.project_db(DEMO_PROJECT_ID);
    assert_eq!(
        query_string(&conn, "SELECT value FROM metadata WHERE key = 'region'"),
        "eu"
    );
    Ok(())
}

#[tokio::test]
async fn session_merge_on_shared_user_id() -> io::Result<()> {
    let mut server = TestServer::spawn("session-merge").await?;
    let a = TrackHit::pageview(DEMO_PROJECT_NAME, "/", &page_id(1))
        .with_metadata(json!({"user_id": "user-42"}));
    let a_json: IngestJson = server
        .post_ingest(
            &a.to_form(),
            &[
                ("X-Forwarded-For", "198.51.100.1"),
                ("User-Agent", CHROME_UA),
            ],
        )
        .await?
        .json();
    server.flush().await;

    let b = TrackHit::pageview(DEMO_PROJECT_NAME, "/welcome", &page_id(2))
        .with_metadata(json!({"user_id": "user-42"}));
    let b_json: IngestJson = server
        .post_ingest(
            &b.to_form(),
            &[
                ("X-Forwarded-For", "198.51.100.2"),
                ("User-Agent", FIREFOX_UA),
            ],
        )
        .await?
        .json();
    assert_ne!(a_json.visitor_timestamp_id, b_json.visitor_timestamp_id);
    server.flush().await;

    let conn = server.project_db(DEMO_PROJECT_ID);
    let distinct_visitors = query_i64(
        &conn,
        "SELECT COUNT(DISTINCT visitor_timestamp_id) FROM pageviews",
    );
    assert_eq!(distinct_visitors, 1);
    assert_eq!(
        query_i64(
            &conn,
            "SELECT visitor_timestamp_id FROM pageviews ORDER BY timestamp_id LIMIT 1"
        ),
        a_json.visitor_timestamp_id.unwrap()
    );
    assert_eq!(count_rows(&conn, "landings"), 1);
    assert_eq!(
        query_i64(&conn, "SELECT session_page_count FROM landings"),
        2
    );
    Ok(())
}

#[tokio::test]
async fn gif_get_fallback_persists_identically() -> io::Result<()> {
    let mut server = TestServer::spawn("gif-fallback").await?;
    let hit = TrackHit::pageview(DEMO_PROJECT_NAME, "/gif", &page_id(1));
    let response = server.get_gif(&hit.to_form(), &[]).await?;
    assert_eq!(response.status_code(), 200);
    assert_eq!(response.header("content-type"), Some("image/gif"));
    assert_eq!(response.body.len(), 42);
    server.flush().await;

    let conn = server.project_db(DEMO_PROJECT_ID);
    assert_eq!(count_rows(&conn, "pageviews"), 1);
    assert_eq!(query_string(&conn, "SELECT path FROM paths"), "/gif");
    Ok(())
}

#[tokio::test]
async fn beacon_style_batch_post_persists() -> io::Result<()> {
    let mut server = TestServer::spawn("beacon-batch").await?;
    let page = TrackHit::pageview(DEMO_PROJECT_NAME, "/", &page_id(1));
    let engagement = TrackHit::engagement(DEMO_PROJECT_NAME, &page_id(1), 9, 25);
    let body = batch_form(DEMO_PROJECT_NAME, &[page, engagement]);
    let response = server.post_ingest(&body, &[]).await?;
    assert_eq!(response.status_code(), 200);
    server.flush().await;

    let conn = server.project_db(DEMO_PROJECT_ID);
    assert_eq!(count_rows(&conn, "pageviews"), 1);
    assert_eq!(
        query_i64(&conn, "SELECT duration_seconds FROM pageviews"),
        9
    );
    assert_eq!(query_i64(&conn, "SELECT consent FROM landings"), 1);
    Ok(())
}

#[tokio::test]
async fn route_and_protocol_edges() -> io::Result<()> {
    let mut server = TestServer::spawn("protocol-edges").await?;

    // Unmatched routes return 200 with a gzipped 404 body aimed at bots probing
    // non-existent URLs, not an empty HTTP 404.
    let invalid_path = server
        .post_form("/not-a-valid-name", "ty=pageview&p=/&pg=zz", &[])
        .await?;
    assert_eq!(invalid_path.status_code(), 200);
    assert_eq!(invalid_path.header("content-encoding"), Some("gzip"));

    let unknown_project = server
        .post_ingest_for(
            "missing_project",
            &TrackHit::pageview("missing_project", "/", &page_id(99)).to_form(),
            &[],
        )
        .await?;
    assert_eq!(unknown_project.status_code(), 404);

    let invalid_payload = server.post_ingest("ty=engagement&d=10&sc=80", &[]).await?;
    assert_eq!(invalid_payload.status_code(), 400);

    let unknown_type = TrackHit {
        hit_type: Some("consent"),
        time: Some(1),
        version: Some(1.0),
        hostname: Some("example.com".to_string()),
        path: Some("/".to_string()),
        page_id: Some(page_id(1)),
        ..TrackHit::default()
    };
    let ignored = server.post_ingest(&unknown_type.to_form(), &[]).await?;
    assert_eq!(ignored.status_code(), 200);

    let cors = server
        .post_ingest(
            &TrackHit::pageview(DEMO_PROJECT_NAME, "/", &page_id(2)).to_form(),
            &[("Origin", "https://customer.example.com")],
        )
        .await?;
    assert_eq!(cors.status_code(), 200);
    assert_eq!(cors.header("access-control-allow-origin"), Some("*"));
    assert_eq!(cors.header("access-control-allow-credentials"), None);

    let oversized = "x".repeat(140 * 1024);
    match server.post_ingest(&oversized, &[]).await {
        Ok(too_large) => assert_eq!(too_large.status_code(), 413),
        Err(error) => assert!(
            matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe
            ),
            "unexpected oversized-request error: {error}"
        ),
    }

    server.flush().await;
    let conn = server.project_db(DEMO_PROJECT_ID);
    // Only the valid CORS pageview should have been enqueued.
    assert_eq!(count_rows(&conn, "pageviews"), 1);
    Ok(())
}

#[tokio::test]
async fn threshold_auto_flush_persists_without_manual_flush() -> io::Result<()> {
    let mut server = TestServer::spawn("threshold-flush").await?;
    // BATCH_FLUSH_THRESHOLD is 1000. Send 50 batches of 20 pageviews (=1000) to
    // force an automatic flush, then one more pageview that stays buffered.
    for batch_idx in 0..50 {
        let mut items = Vec::with_capacity(20);
        for item_idx in 0..20 {
            let n = batch_idx * 20 + item_idx + 1;
            let pg = format!("{n:024x}");
            items.push(TrackHit::pageview(
                DEMO_PROJECT_NAME,
                &format!("/p{n}"),
                &pg,
            ));
        }
        let response = server
            .post_ingest(&batch_form(DEMO_PROJECT_NAME, &items), &[])
            .await?;
        assert_eq!(
            response.status_code(),
            200,
            "batch {batch_idx} failed: {}",
            response.body_str()
        );
    }

    // The threshold flush runs on a background task after the response. Wait
    // on the queue rather than polling the database: in the shared-cache test
    // database a read locks the table it reads, and the flush cannot write
    // pageviews while that lock is held.
    let project_key: i32 = DEMO_PROJECT_ID.parse().expect("demo project id");
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    while server.context.has_queued_ingest(project_key) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "threshold flush did not write the queued pageviews"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    {
        let conn = server.project_db(DEMO_PROJECT_ID);
        assert_eq!(
            count_rows(&conn, "pageviews"),
            1000,
            "threshold flush should persist exactly 1000 pageviews"
        );
    }

    let extra = TrackHit::pageview(DEMO_PROJECT_NAME, "/extra", &format!("{:024x}", 10_001));
    server.post_ingest(&extra.to_form(), &[]).await?;
    {
        let conn = server.project_db(DEMO_PROJECT_ID);
        assert_eq!(
            count_rows(&conn, "pageviews"),
            1000,
            "extra pageview should still be buffered"
        );
    }

    server.flush().await;
    let conn = server.project_db(DEMO_PROJECT_ID);
    assert_eq!(count_rows(&conn, "pageviews"), 1001);
    let distinct_ts = query_i64(&conn, "SELECT COUNT(DISTINCT timestamp_id) FROM pageviews");
    assert_eq!(distinct_ts, 1001);
    Ok(())
}

#[tokio::test]
async fn multi_project_isolation() -> io::Result<()> {
    let mut server = TestServer::spawn("multi-project").await?;
    server
        .post_ingest(
            &TrackHit::pageview(DEMO_PROJECT_NAME, "/a", &page_id(1)).to_form(),
            &[],
        )
        .await?;
    server
        .post_ingest_for(
            DEMO_PROJECT_NAME_B,
            &TrackHit::pageview(DEMO_PROJECT_NAME_B, "/b", &page_id(1)).to_form(),
            &[],
        )
        .await?;
    server.flush().await;

    {
        let conn_a = server.project_db(DEMO_PROJECT_ID);
        assert_eq!(count_rows(&conn_a, "pageviews"), 1);
        assert_eq!(query_string(&conn_a, "SELECT path FROM paths"), "/a");
    }
    {
        let conn_b = server.project_db(DEMO_PROJECT_ID_B);
        assert_eq!(count_rows(&conn_b, "pageviews"), 1);
        assert_eq!(query_string(&conn_b, "SELECT path FROM paths"), "/b");
    }
    Ok(())
}

#[tokio::test]
async fn duplicate_pageview_in_the_same_batch_reuses_the_existing_page() -> io::Result<()> {
    let mut server = TestServer::spawn("dup-pageview-batch").await?;
    let first = TrackHit::pageview(DEMO_PROJECT_NAME, "/faq/", &page_id(1));
    let first_json: IngestJson = server
        .post_ingest(&first.to_form(), &[("User-Agent", CHROME_UA)])
        .await?
        .json();

    let retry = TrackHit::pageview(DEMO_PROJECT_NAME, "/faq/", &page_id(1));
    let retry_json: IngestJson = server
        .post_ingest(&retry.to_form(), &[("User-Agent", FIREFOX_UA)])
        .await?
        .json();

    assert_eq!(
        retry_json.visitor_timestamp_id,
        first_json.visitor_timestamp_id
    );

    server.flush().await;
    let conn = server.project_db(DEMO_PROJECT_ID);
    assert_eq!(count_rows(&conn, "pageviews"), 1);
    assert_eq!(count_rows(&conn, "landings"), 1);
    assert_eq!(count_rows(&conn, "client_page_ids_map"), 1);
    Ok(())
}

#[tokio::test]
async fn duplicate_pageview_after_flush_reuses_the_mapped_page() -> io::Result<()> {
    let mut server = TestServer::spawn("dup-pageview-flushed").await?;
    let first = TrackHit::pageview(DEMO_PROJECT_NAME, "/faq/", &page_id(1));
    let first_json: IngestJson = server
        .post_ingest(&first.to_form(), &[("User-Agent", CHROME_UA)])
        .await?
        .json();
    server.flush().await;

    let retry = TrackHit::pageview(DEMO_PROJECT_NAME, "/faq/", &page_id(1));
    let retry_json: IngestJson = server
        .post_ingest(&retry.to_form(), &[("User-Agent", FIREFOX_UA)])
        .await?
        .json();
    assert_eq!(
        retry_json.visitor_timestamp_id,
        first_json.visitor_timestamp_id
    );

    let event = TrackHit::event(DEMO_PROJECT_NAME, "faq-expand", &page_id(1));
    server
        .post_ingest(&event.to_form(), &[("User-Agent", FIREFOX_UA)])
        .await?;
    server.flush().await;

    let conn = server.project_db(DEMO_PROJECT_ID);
    assert_eq!(count_rows(&conn, "pageviews"), 1);
    assert_eq!(count_rows(&conn, "landings"), 1);
    assert_eq!(count_rows(&conn, "client_page_ids_map"), 1);
    assert_eq!(count_rows(&conn, "events"), 1);
    Ok(())
}

#[tokio::test]
async fn flush_is_idempotent() -> io::Result<()> {
    let mut server = TestServer::spawn("flush-idempotent").await?;
    server.flush().await;

    server
        .post_ingest(
            &TrackHit::pageview(DEMO_PROJECT_NAME, "/", &page_id(1)).to_form(),
            &[],
        )
        .await?;
    server.flush().await;
    server.flush().await;

    let conn = server.project_db(DEMO_PROJECT_ID);
    assert_eq!(count_rows(&conn, "pageviews"), 1);
    assert_eq!(count_rows(&conn, "landings"), 1);
    Ok(())
}

#[tokio::test]
async fn error_hit_goes_to_postgres_when_configured() -> io::Result<()> {
    let server = TestServer::spawn("error-hit").await?;

    let mut hit = TrackHit::pageview(DEMO_PROJECT_NAME, "/broken", &page_id(1));
    hit.hit_type = Some("error");
    hit.error = Some("TypeError: boom".to_string());
    let response = server.post_ingest(&hit.to_form(), &[]).await?;
    assert_eq!(response.status_code(), 200);

    let conn = revscope::db::open_main_database_read(server.context.clone())
        .await
        .map_err(io::Error::other)?;
    let project_id: i32 = DEMO_PROJECT_ID.parse().expect("demo project id");
    let count: i64 = conn
        .query_one(
            "SELECT COUNT(*) FROM errors WHERE project_id = $1 AND error_text = $2",
            &[&project_id, &"TypeError: boom"],
        )
        .await
        .map_err(io::Error::other)?
        .get(0);
    assert!(count >= 1, "expected error row in main database");
    Ok(())
}

/// Posts a tracker error report for `/broken` with `text` as the message.
async fn post_error_report(server: &TestServer, text: &str) -> io::Result<()> {
    post_error_report_on(server, "/broken", text).await
}

async fn post_error_report_on(server: &TestServer, path: &str, text: &str) -> io::Result<()> {
    let mut hit = TrackHit::pageview(DEMO_PROJECT_NAME, path, &page_id(1));
    hit.hit_type = Some("error");
    hit.error = Some(text.to_string());
    let response = server.post_ingest(&hit.to_form(), &[]).await?;
    assert_eq!(response.status_code(), 200);
    Ok(())
}

/// `(count, error_text)` of every javascript error row whose text starts with
/// `prefix`. Tests share the demo project in one postgres, so each test picks a
/// prefix unique to its run.
async fn javascript_error_rows(
    server: &TestServer,
    prefix: &str,
) -> io::Result<Vec<(i64, String)>> {
    let conn = revscope::db::open_main_database_read(server.context.clone())
        .await
        .map_err(io::Error::other)?;
    let project_id: i32 = DEMO_PROJECT_ID.parse().expect("demo project id");
    let rows = conn
        .query(
            "SELECT count, error_text FROM errors
             WHERE project_id = $1 AND error_type = 'javascript'
             AND starts_with(error_text, $2)
             ORDER BY error_text",
            &[&project_id, &prefix],
        )
        .await
        .map_err(io::Error::other)?;
    Ok(rows.iter().map(|row| (row.get(0), row.get(1))).collect())
}

#[tokio::test]
async fn a_repeated_error_increments_one_row_per_day() -> io::Result<()> {
    let server = TestServer::spawn("error-dedupe").await?;
    let prefix = format!("dedupe-{}", unix_millis());
    let repeated = format!("{prefix} TypeError: boom");
    let other = format!("{prefix} RangeError: other");

    post_error_report(&server, &repeated).await?;
    // The same error on another page is still the same error.
    post_error_report_on(&server, "/elsewhere", &repeated).await?;
    post_error_report(&server, &other).await?;

    assert_eq!(
        javascript_error_rows(&server, &prefix).await?,
        vec![(1, other), (2, repeated)]
    );
    Ok(())
}

#[tokio::test]
async fn an_oversized_error_report_is_truncated() -> io::Result<()> {
    let server = TestServer::spawn("error-truncate").await?;
    let prefix = format!("truncate-{}", unix_millis());
    post_error_report(&server, &format!("{prefix}{}", "é".repeat(10_000))).await?;

    let rows = javascript_error_rows(&server, &prefix).await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1.chars().count(), 4096);
    Ok(())
}

#[tokio::test]
async fn error_reports_past_the_per_ip_burst_are_dropped() -> io::Result<()> {
    let server = TestServer::spawn("error-rate-limit").await?;
    let prefix = format!("limit-{}", unix_millis());
    for attempt in 0..40 {
        post_error_report(&server, &format!("{prefix} {attempt:02}")).await?;
    }
    // The burst is 30, plus one more for each 10 seconds the posts took.
    let recorded = javascript_error_rows(&server, &prefix).await?.len();
    assert!((30..=32).contains(&recorded), "recorded {recorded} of 40");
    Ok(())
}

#[tokio::test]
async fn cleanup_deletes_errors_older_than_sixty_days() -> io::Result<()> {
    let server = TestServer::spawn("error-cleanup").await?;
    let conn = revscope::db::open_main_database_write(server.context.clone())
        .await
        .map_err(io::Error::other)?;
    let project_id: i32 = DEMO_PROJECT_ID.parse().expect("demo project id");
    let marker = format!("cleanup-{}", unix_millis());
    let day_millis = 24 * 60 * 60 * 1000;
    // Row ids are UUIDv7, which is what dates a row for the cleanup.
    let v7_at = |millis: i64| format!("{millis:012x}7000{:016x}", 0x8000_0000_0000_0000_u64);
    for (age_days, label) in [(61, "old"), (59, "recent")] {
        conn.execute(
            "INSERT INTO errors (id, project_id, error_type, error_text)
             VALUES ($1::text::uuid, $2, 'cleanup_test', $3)",
            &[
                &v7_at(unix_millis() - age_days * day_millis),
                &project_id,
                &format!("{marker} {label}"),
            ],
        )
        .await
        .map_err(io::Error::other)?;
    }

    revscope::run_main_db_cleanup(server.context.clone()).await;

    let remaining: Vec<String> = conn
        .query(
            "SELECT error_text FROM errors WHERE starts_with(error_text, $1)",
            &[&marker],
        )
        .await
        .map_err(io::Error::other)?
        .iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(remaining, vec![format!("{marker} recent")]);
    Ok(())
}

#[tokio::test]
async fn check_constraint_failure_skips_enqueue_and_records_server_error() -> io::Result<()> {
    let mut server = TestServer::spawn("constraint-skip").await?;

    let mut empty_key_meta = serde_json::Map::new();
    empty_key_meta.insert("".to_string(), json!("x"));
    let hit = TrackHit::pageview(DEMO_PROJECT_NAME, "/bad-meta", &page_id(1))
        .with_metadata(serde_json::Value::Object(empty_key_meta));

    let response = server.post_ingest(&hit.to_form(), &[]).await?;
    assert_eq!(response.status_code(), 200);
    let json: IngestJson = response.json();
    assert!(json.visitor_timestamp_id.is_none());

    server.flush().await;
    {
        let conn = server.project_db(DEMO_PROJECT_ID);
        assert_eq!(count_rows(&conn, "pageviews"), 0);
        assert_eq!(count_rows(&conn, "landings"), 0);
        assert_eq!(count_rows(&conn, "metadata"), 0);
    }

    let conn = revscope::db::open_main_database_read(server.context.clone())
        .await
        .map_err(io::Error::other)?;
    let project_id: i32 = DEMO_PROJECT_ID.parse().expect("demo project id");
    // Tests share the demo project in one postgres, so match this payload rather
    // than whichever constraint failure was recorded last.
    let row = conn
        .query_one(
            "SELECT error_type, is_server_error, error_text, path, data
             FROM errors
             WHERE project_id = $1 AND error_type = $2 AND (data::jsonb ->> 'p') = $3
             ORDER BY id DESC
             LIMIT 1",
            &[&project_id, &"check_constraint", &"/bad-meta"],
        )
        .await
        .map_err(io::Error::other)?;
    let error_type: String = row.get(0);
    let is_server_error: bool = row.get(1);
    let error_text: Option<String> = row.get(2);
    let path: Option<String> = row.get(3);
    let data: Option<String> = row.get(4);
    assert_eq!(error_type, "check_constraint");
    assert!(is_server_error);
    assert!(
        error_text
            .as_deref()
            .is_some_and(|text| text.contains("metadata.key")),
        "unexpected error_text: {error_text:?}"
    );
    assert!(path.is_none());
    let data_json: serde_json::Value =
        serde_json::from_str(data.as_deref().expect("data")).map_err(io::Error::other)?;
    assert_eq!(data_json["p"], "/bad-meta");
    Ok(())
}

#[tokio::test]
async fn check_constraint_failure_does_not_block_sibling_batch_items() -> io::Result<()> {
    let mut server = TestServer::spawn("constraint-sibling").await?;
    let mut empty_key_meta = serde_json::Map::new();
    empty_key_meta.insert("".to_string(), json!("x"));
    let bad = TrackHit::pageview(DEMO_PROJECT_NAME, "/bad", &page_id(1))
        .with_metadata(serde_json::Value::Object(empty_key_meta));
    let good = TrackHit::pageview(DEMO_PROJECT_NAME, "/good", &page_id(2));

    let response = server
        .post_ingest(&batch_form(DEMO_PROJECT_NAME, &[bad, good]), &[])
        .await?;
    assert_eq!(response.status_code(), 200);
    server.flush().await;

    let conn = server.project_db(DEMO_PROJECT_ID);
    assert_eq!(count_rows(&conn, "pageviews"), 1);
    assert_eq!(query_string(&conn, "SELECT path FROM paths"), "/good");
    Ok(())
}

/// The `unknown_page` error recorded for the event named `event_name`. Dropped
/// orphan events are recorded by a task the flush spawns, so this polls.
async fn wait_for_unknown_page_error(
    server: &TestServer,
    event_name: &str,
) -> io::Result<(String, Option<String>)> {
    let main = revscope::db::open_main_database_read(server.context.clone())
        .await
        .map_err(io::Error::other)?;
    let project_id: i32 = DEMO_PROJECT_ID.parse().expect("demo project id");
    for _ in 0..100 {
        let row = main
            .query_opt(
                "SELECT error_type, error_text
                 FROM errors
                 WHERE project_id = $1 AND error_type = $2 AND (data::jsonb ->> 'ev') = $3
                 ORDER BY id DESC
                 LIMIT 1",
                &[&project_id, &"unknown_page", &event_name],
            )
            .await
            .map_err(io::Error::other)?;
        if let Some(row) = row {
            return Ok((row.get(0), row.get(1)));
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    Err(io::Error::other(format!(
        "no unknown_page error recorded for {event_name}"
    )))
}

#[tokio::test]
async fn unknown_page_event_soft_drops_without_failing_batch() -> io::Result<()> {
    let mut server = TestServer::spawn("unknown-page-event").await?;
    // Tests share the demo project in one postgres, so the event name has to
    // pick out this run's error row.
    let orphan_name = format!("orphan-{}", unix_millis());
    let page = TrackHit::pageview(DEMO_PROJECT_NAME, "/", &page_id(1));
    let orphan = TrackHit::event(DEMO_PROJECT_NAME, &orphan_name, &page_id(9));
    let sibling = TrackHit::event(DEMO_PROJECT_NAME, "ok", &page_id(1));

    let response = server
        .post_ingest(
            &batch_form(DEMO_PROJECT_NAME, &[page, orphan, sibling]),
            &[],
        )
        .await?;
    assert_eq!(response.status_code(), 200);
    server.flush().await;

    let conn = server.project_db(DEMO_PROJECT_ID);
    assert_eq!(count_rows(&conn, "pageviews"), 1);
    assert_eq!(count_rows(&conn, "events"), 1);
    assert_eq!(
        query_string(
            &conn,
            "SELECT name FROM event_names JOIN events ON events.event_name_id = event_names.id"
        ),
        "ok"
    );

    let (error_type, error_text) = wait_for_unknown_page_error(&server, &orphan_name).await?;
    assert_eq!(error_type, "unknown_page");
    assert!(
        error_text
            .as_deref()
            .is_some_and(|text| text.contains("no resolvable pageview")),
        "unexpected error_text: {error_text:?}"
    );
    Ok(())
}

#[tokio::test]
async fn event_sent_ahead_of_its_pageview_is_kept() -> io::Result<()> {
    let mut server = TestServer::spawn("event-ahead-of-pageview").await?;
    // The tracker sends these as two requests, and the event can arrive first.
    let event = TrackHit::event(DEMO_PROJECT_NAME, "saw-hero", &page_id(1));
    let response = server.post_ingest(&event.to_form(), &[]).await?;
    assert_eq!(response.status_code(), 200);
    let page = TrackHit::pageview(DEMO_PROJECT_NAME, "/", &page_id(1));
    let response = server.post_ingest(&page.to_form(), &[]).await?;
    assert_eq!(response.status_code(), 200);
    server.flush().await;

    let conn = server.project_db(DEMO_PROJECT_ID);
    assert_eq!(count_rows(&conn, "pageviews"), 1);
    assert_eq!(
        query_string(
            &conn,
            "SELECT name FROM event_names JOIN events ON events.event_name_id = event_names.id"
        ),
        "saw-hero"
    );
    assert_eq!(
        query_i64(
            &conn,
            "SELECT COUNT(*) FROM events JOIN pageviews
             ON pageviews.timestamp_id = events.page_timestamp_id
             WHERE events.timestamp_id > pageviews.timestamp_id"
        ),
        1
    );
    Ok(())
}
