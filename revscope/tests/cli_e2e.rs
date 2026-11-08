//! Runs the real `revscope` CLI binary against a real server, so the two can't drift
//! apart without a test failing: exit statuses, `--json` output, what it stores on
//! disk, and whether what it sends means what it thinks it means.
//!
//! Hosted only: self-hosted signup closes after the first user in the shared database.
#![cfg(not(feature = "self-hosted"))]

mod common;

use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Once;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use common::signup::{NewSignup, SIGNUP_PATH, delete_accounts, unique_slug};
use common::{CHROME_UA, TestServer, TrackHit, page_id};
use serde_json::{Value, json};

const CLI_MANIFEST: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../cli/Cargo.toml");
const CLI_TARGET: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../cli/target");
const PASSWORD: &str = "a-long-enough-password";

static BUILD: Once = Once::new();
static CONFIG_COUNTER: AtomicU32 = AtomicU32::new(0);

fn cli_binary() -> PathBuf {
    BUILD.call_once(|| {
        let status = Command::new(env!("CARGO"))
            .args([
                "build",
                "--locked",
                "--quiet",
                "--manifest-path",
                CLI_MANIFEST,
            ])
            .args(["--target-dir", CLI_TARGET])
            .status()
            .expect("run cargo build for the CLI");
        assert!(status.success(), "the CLI failed to build");
        // SAFETY: inside the `Once`, before any server in this binary saves an integration.
        unsafe { std::env::set_var("INTEGRATION_ENCRYPTION_KEY", "11".repeat(32)) };
    });
    Path::new(CLI_TARGET).join("debug/revscope")
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Run {
    fn json(&self) -> Value {
        serde_json::from_str(&self.stdout)
            .unwrap_or_else(|error| panic!("not JSON ({error}): {}", self.stdout))
    }

    #[track_caller]
    fn assert_ok(&self) -> &Self {
        assert_eq!(
            self.code, 0,
            "stdout: {}\nstderr: {}",
            self.stdout, self.stderr
        );
        self
    }

    #[track_caller]
    fn assert_exit(&self, code: i32) -> &Self {
        assert_eq!(
            self.code, code,
            "stdout: {}\nstderr: {}",
            self.stdout, self.stderr
        );
        self
    }
}

/// One user of the CLI: a config directory of its own, against one server.
struct Cli {
    api_url: String,
    config_dir: PathBuf,
    token: Option<String>,
}

impl Cli {
    fn new(server: &TestServer) -> Self {
        cli_binary();
        let config_dir = std::env::temp_dir().join(format!(
            "revscope-cli-e2e-{}-{}",
            std::process::id(),
            CONFIG_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&config_dir);
        Self {
            api_url: format!("http://{}", server.addr),
            config_dir,
            token: None,
        }
    }

    fn credentials_file(&self) -> PathBuf {
        self.config_dir.join("credentials.json")
    }

    async fn run(&self, args: &[&str]) -> Run {
        self.run_with_stdin(args, "").await
    }

    /// Off the runtime, since the server under test is on it.
    async fn run_with_stdin(&self, args: &[&str], stdin: &str) -> Run {
        let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
        let stdin = stdin.to_string();
        let api_url = self.api_url.clone();
        let config_dir = self.config_dir.clone();
        let token = self.token.clone();
        tokio::task::spawn_blocking(move || {
            let mut command = Command::new(cli_binary());
            command
                .args(&args)
                .env("REVSCOPE_API_URL", api_url)
                .env("REVSCOPE_CONFIG_DIR", config_dir)
                .env_remove("REVSCOPE_PROJECT")
                .env_remove("REVSCOPE_PASSWORD")
                .env_remove("REVSCOPE_TOKEN")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            if let Some(token) = token {
                command.env("REVSCOPE_TOKEN", token);
            }
            let mut child = command.spawn().expect("start the CLI");
            child
                .stdin
                .take()
                .expect("stdin")
                .write_all(stdin.as_bytes())
                .expect("write stdin");
            let output = child.wait_with_output().expect("wait for the CLI");
            Run {
                code: output.status.code().expect("exit status"),
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            }
        })
        .await
        .expect("join")
    }

    /// Signs up a new account with no password, which is what an agent does. Returns
    /// the account's name, for cleanup.
    async fn signup(&self, tag: &str) -> Account {
        let slug = unique_slug(tag);
        let website = format!("https://{slug}.example");
        let email = format!("{slug}@example.com");
        let run = self
            .run(&[
                "--json",
                "signup",
                "--name",
                "Ada Agent",
                "--email",
                &email,
                "--website",
                &website,
            ])
            .await;
        run.assert_ok();
        let body = run.json();
        let project = body["projects"]
            .as_object()
            .expect("projects")
            .keys()
            .next()
            .expect("a first project")
            .clone();
        Account {
            account_name: format!("{slug}.example"),
            project,
        }
    }
}

impl Drop for Cli {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.config_dir);
    }
}

struct Account {
    account_name: String,
    project: String,
}

#[tokio::test]
async fn an_agent_signs_up_with_no_password_and_works_from_the_stored_token() -> io::Result<()> {
    let server = TestServer::spawn_on_disk("cli-signup").await?;
    let cli = Cli::new(&server);
    let slug = unique_slug("cli-signup");
    let email = format!("{slug}@example.com");
    let website = format!("https://{slug}.example");

    let run = cli
        .run(&[
            "signup",
            "--name",
            "Ada Agent",
            "--email",
            &email,
            "--website",
            &website,
        ])
        .await;
    run.assert_ok();
    assert!(run.stdout.contains("Created the account"), "{}", run.stdout);
    assert!(run.stdout.contains("password-reset"), "{}", run.stdout);

    let stored = std::fs::read_to_string(cli.credentials_file())?;
    let stored: Value = serde_json::from_str(&stored)?;
    let token = stored["tokens"][&cli.api_url]
        .as_str()
        .expect("a stored token");
    assert!(token.starts_with("rsc_"), "{token}");
    assert!(!run.stdout.contains(token), "the token must not be printed");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(cli.credentials_file())?
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    let me = cli.run(&["--json", "whoami"]).await;
    me.assert_ok();
    let me = me.json();
    assert_eq!(me["email"], json!(email));
    assert!(me.get("api_token").is_none(), "{me}");
    let project_name = me["projects"]
        .as_object()
        .unwrap()
        .keys()
        .next()
        .unwrap()
        .clone();
    assert!(me["projects"][&project_name]["id"].is_string());

    // The token lasts a year, not the 30 days of a browser login.
    let conn = common::signup::main_db(&server).await;
    let days: f64 = conn
        .query_one(
            "SELECT EXTRACT(EPOCH FROM (s.expires_at - NOW()))::float8 / 86400
             FROM sessions s JOIN users u ON u.id = s.user_id
             WHERE u.email = $1",
            &[&email],
        )
        .await
        .expect("the session row")
        .get(0);
    assert!(days > 360.0, "{days}");
    drop(conn);

    // A second signup here would replace the only way into this account.
    let second = cli
        .run(&[
            "signup",
            "--name",
            "Ada Agent",
            "--email",
            "second@example.com",
            "--website",
            "https://second.example",
        ])
        .await;
    second.assert_exit(6);
    assert!(second.stderr.contains(&email), "{}", second.stderr);
    assert_eq!(stored_token(&cli), token);

    // So would another's, for the same website, and it says how to get in.
    let other = Cli::new(&server);
    let taken = other
        .run(&[
            "signup",
            "--name",
            "Ada Agent",
            "--email",
            "other@example.com",
            "--website",
            &website,
        ])
        .await;
    taken.assert_exit(6);
    assert!(taken.stderr.contains("password-reset"), "{}", taken.stderr);

    // With a token in the environment, what a signup stores would be ignored.
    let mut with_env = Cli::new(&server);
    with_env.token = Some(token.to_string());
    with_env
        .run(&[
            "signup",
            "--name",
            "Ada",
            "--email",
            "third@example.com",
            "--website",
            "https://third.example",
        ])
        .await
        .assert_exit(6);

    delete_accounts(&server, &[&format!("{slug}.example")]).await;
    Ok(())
}

#[tokio::test]
async fn logout_revokes_the_token_on_the_server() -> io::Result<()> {
    let server = TestServer::spawn_on_disk("cli-logout").await?;
    let mut cli = Cli::new(&server);
    let account = cli.signup("cli-logout").await;
    let stored: Value = serde_json::from_slice(&std::fs::read(cli.credentials_file())?)?;
    let token = stored["tokens"][&cli.api_url].as_str().unwrap().to_string();

    cli.run(&["logout"]).await.assert_ok();
    cli.run(&["whoami"]).await.assert_exit(3);

    // A failure under --json parses like a success, with what a script branches on.
    let signed_out = cli.run(&["--json", "whoami"]).await;
    signed_out.assert_exit(3);
    let failure = signed_out.json();
    assert_eq!(
        failure["error"]["code"],
        json!("not_authenticated"),
        "{failure}"
    );
    assert_eq!(failure["error"]["status"], json!(401));
    assert_eq!(failure["error"]["exit_code"], json!(3));

    // Not just forgotten here: the token itself no longer works.
    cli.token = Some(token);
    let refused = cli.run(&["whoami"]).await;
    refused.assert_exit(3);
    assert!(
        refused.stderr.contains("revscope login"),
        "{}",
        refused.stderr
    );

    delete_accounts(&server, &[&account.account_name]).await;
    Ok(())
}

#[tokio::test]
async fn login_with_a_password_and_a_token_from_the_environment() -> io::Result<()> {
    let server = TestServer::spawn_on_disk("cli-login").await?;
    let cli = Cli::new(&server);
    let slug = unique_slug("cli-login");
    let email = format!("{slug}@example.com");
    let website = format!("https://{slug}.example");
    cli.run_with_stdin(
        &[
            "signup",
            "--name",
            "Ada",
            "--email",
            &email,
            "--website",
            &website,
            "--password-stdin",
        ],
        &format!("{PASSWORD}\n"),
    )
    .await
    .assert_ok();
    cli.run(&["logout"]).await.assert_ok();

    let wrong = cli
        .run_with_stdin(
            &["login", "--email", &email, "--password-stdin"],
            "not-the-password\n",
        )
        .await;
    wrong.assert_exit(3);
    assert!(
        wrong.stderr.contains("wrong email or password"),
        "{}",
        wrong.stderr
    );

    cli.run_with_stdin(
        &["login", "--email", &email, "--password-stdin"],
        &format!("{PASSWORD}\n"),
    )
    .await
    .assert_ok();
    cli.run(&["whoami"]).await.assert_ok();

    // A credential from the environment wins over the stored one, and a dead one fails
    // rather than quietly falling back.
    let mut with_bad_env = Cli::new(&server);
    with_bad_env.config_dir = cli.config_dir.clone();
    with_bad_env.token = Some("rsc_00000000-0000-7000-8000-000000000000".to_string());
    with_bad_env.run(&["whoami"]).await.assert_exit(3);
    // Sharing the directory, so don't let this one's cleanup remove it first.
    with_bad_env.config_dir = std::env::temp_dir().join("revscope-cli-e2e-unused");

    delete_accounts(&server, &[&format!("{slug}.example")]).await;
    Ok(())
}

#[tokio::test]
async fn queries_tables_and_schema_read_the_real_database() -> io::Result<()> {
    let server = TestServer::spawn_on_disk("cli-query").await?;
    let cli = Cli::new(&server);
    let account = cli.signup("cli-query").await;

    let tables = cli.run(&["--json", "tables"]).await;
    tables.assert_ok();
    let names = tables.json();
    assert!(
        names["tables"]
            .as_array()
            .unwrap()
            .iter()
            .any(|name| name == "pageviews"),
        "{names}"
    );

    let schema = cli.run(&["schema", "--table", "pageviews"]).await;
    schema.assert_ok();
    assert!(
        schema.stdout.contains("CREATE TABLE pageviews"),
        "{}",
        schema.stdout
    );
    cli.run(&["schema"]).await.assert_ok();
    cli.run(&["schema", "--table", "no_such_table"])
        .await
        .assert_exit(2);
    cli.run(&["schema", "--table", "events'; --"])
        .await
        .assert_exit(2);

    let query = cli
        .run(&["--json", "query", "SELECT 1 AS one, 'two' AS second"])
        .await;
    query.assert_ok();
    let result = query.json();
    assert_eq!(result["columns"], json!(["one", "second"]));
    assert_eq!(result["rows"], json!([[1, "two"]]));

    // All of stdin, not its first line.
    let multi_line = cli
        .run_with_stdin(
            &["--json", "query", "-"],
            "SELECT 1 AS a\nUNION ALL\nSELECT 2\nORDER BY a\n",
        )
        .await;
    multi_line.assert_ok();
    assert_eq!(multi_line.json()["rows"], json!([[1], [2]]));

    let every_page = cli
        .run(&[
            "--json", "query", "--all", "--limit", "2",
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 5) SELECT i FROM n ORDER BY i",
        ])
        .await;
    every_page.assert_ok();
    let every_page = every_page.json();
    assert_eq!(every_page["rows"], json!([[1], [2], [3], [4], [5]]));
    assert_eq!(every_page["has_more"], json!(false));

    // Refused by the server, with the bad-input exit status.
    let write = cli.run(&["query", "DELETE FROM pageviews"]).await;
    write.assert_exit(2);
    assert!(write.stderr.contains("SELECT"), "{}", write.stderr);

    delete_accounts(&server, &[&account.account_name]).await;
    Ok(())
}

#[tokio::test]
async fn every_integration_in_the_catalog_can_be_connected_and_disconnected() -> io::Result<()> {
    let server = TestServer::spawn_on_disk("cli-integrations").await?;
    let cli = Cli::new(&server);
    let account = cli.signup("cli-integrations").await;

    let commands: [&[&str]; 8] = [
        &["stripe", "--secret-key", "sk_test_0000000000"],
        &[
            "shopify",
            "--shop",
            "a.myshopify.com",
            "--admin-token",
            "shpat_0000000000",
        ],
        &["square", "--access-token", "square_0000000000"],
        &["lemon-squeezy", "--api-key", "lemon_0000000000"],
        &["polar", "--access-token", "polar_oat_0000000000"],
        &["dodo", "--api-key", "dodo_0000000000"],
        &[
            "search-console",
            "--property",
            "sc-domain:a.example",
            "--service-account-email",
            "a@b.iam.gserviceaccount.com",
            "--private-key",
            "pem_0000000000",
        ],
        &["cloudflare", "--api-token", "cf_0000000000"],
    ];
    assert_eq!(
        commands.len(),
        revscope_api::integrations::INTEGRATIONS.len()
    );
    for command in commands {
        let mut args = vec!["integrations", "connect"];
        args.extend_from_slice(command);
        cli.run(&args).await.assert_ok();
    }

    let listed = cli.run(&["--json", "integrations", "list"]).await;
    listed.assert_ok();
    let listed = listed.json();
    let mut types: Vec<&str> = listed["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|view| view["integration_type"].as_str().unwrap())
        .collect();
    types.sort_unstable();
    let mut catalog: Vec<&str> = revscope_api::integrations::INTEGRATIONS
        .iter()
        .map(|integration| integration.id)
        .collect();
    catalog.sort_unstable();
    assert_eq!(types, catalog);

    // Secrets are stored but never come back whole.
    assert!(
        !listed.to_string().contains("sk_test_0000000000"),
        "{listed}"
    );

    // Rotating one key keeps the shop, which the server would otherwise write as NULL.
    cli.run_with_stdin(
        &["integrations", "connect", "shopify", "--admin-token", "-"],
        "shpat_1111111111\n",
    )
    .await
    .assert_ok();
    let shopify = integration_view(&cli, "shopify").await;
    assert_eq!(shopify["external_account_id"], json!("a.myshopify.com"));
    assert!(
        shopify["api_key_masked"]
            .as_str()
            .unwrap()
            .ends_with("1111"),
        "{shopify}"
    );

    // A whole JSON key file, the way Google hands them out, and a PEM over stdin.
    let key_file = cli.config_dir.join("key.json");
    std::fs::create_dir_all(&cli.config_dir)?;
    std::fs::write(
        &key_file,
        json!({ "client_email": "file@b.iam.gserviceaccount.com", "private_key": "line one\nline two\n" }).to_string(),
    )?;
    cli.run(&[
        "integrations",
        "connect",
        "search-console",
        "--service-account-file",
        key_file.to_str().unwrap(),
    ])
    .await
    .assert_ok();
    let console = integration_view(&cli, "google_search_console").await;
    assert_eq!(console["external_account_id"], json!("sc-domain:a.example"));
    assert!(console["secret_masked"].is_string(), "{console}");
    assert_eq!(console["api_key_masked"], json!("file••••••••.com"));

    cli.run(&["integrations", "disconnect", "stripe"])
        .await
        .assert_ok();
    let listed = cli.run(&["--json", "integrations", "list"]).await.json();
    assert!(
        !listed["integrations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|view| view["integration_type"] == "stripe"),
        "a disconnected integration isn't a connection: {listed}"
    );
    cli.run(&["integrations", "disconnect", "mailchimp"])
        .await
        .assert_exit(2);

    // A first connection needs everything; nothing is saved without it.
    cli.run(&["integrations", "disconnect", "shopify"])
        .await
        .assert_ok();
    let incomplete = cli
        .run(&[
            "integrations",
            "connect",
            "shopify",
            "--admin-token",
            "shpat_2222222222",
        ])
        .await;
    incomplete.assert_exit(2);
    assert!(
        incomplete.stderr.contains("Shop domain"),
        "{}",
        incomplete.stderr
    );

    delete_accounts(&server, &[&account.account_name]).await;
    Ok(())
}

async fn integration_view(cli: &Cli, id: &str) -> Value {
    let listed = cli.run(&["--json", "integrations", "list"]).await;
    listed.assert_ok();
    listed.json()["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|view| view["integration_type"] == id)
        .unwrap_or_else(|| panic!("{id} isn't listed"))
        .clone()
}

#[tokio::test]
async fn settings_and_the_install_snippet_reflect_the_server() -> io::Result<()> {
    let server = TestServer::spawn_on_disk("cli-settings").await?;
    let cli = Cli::new(&server);
    let account = cli.signup("cli-settings").await;

    // On a local server, the tracker reports here, not to production.
    let snippet = cli.run(&["--json", "install", "snippet"]).await;
    snippet.assert_ok();
    let snippet = snippet.json();
    assert_eq!(snippet["project"], json!(account.project));
    assert_eq!(snippet["server_url"], json!(format!("{}/", cli.api_url)));
    assert!(
        snippet["snippet"].as_str().unwrap().contains(&format!(
            "(window, \"{}\", \"{}/\")",
            account.project, cli.api_url
        )),
        "{snippet}"
    );

    let set = cli
        .run(&[
            "--json",
            "settings",
            "set",
            "--timezone",
            "Europe/Berlin",
            "--payment-success-path",
            "/thanks",
        ])
        .await;
    set.assert_ok();
    let set = set.json();
    assert_eq!(set["timezone"], json!("Europe/Berlin"));
    assert_eq!(set["payment_success_path"], json!("/thanks"));

    // An unknown zone keeps the current one, and the CLI says so.
    let bad_zone = cli
        .run(&["settings", "set", "--timezone", "Europe/Berln"])
        .await;
    bad_zone.assert_ok();
    assert!(
        bad_zone.stdout.contains("was not recognized"),
        "{}",
        bad_zone.stdout
    );
    let shown = cli.run(&["--json", "settings", "show"]).await.json();
    assert_eq!(shown["timezone"], json!("Europe/Berlin"));

    // An empty value clears, which the API spells differently from JSON null.
    let cleared = cli
        .run(&["--json", "settings", "set", "--payment-success-path", ""])
        .await;
    cleared.assert_ok();
    assert_eq!(cleared.json()["payment_success_path"], json!(null));

    cli.run(&["settings", "set"]).await.assert_exit(2);

    delete_accounts(&server, &[&account.account_name]).await;
    Ok(())
}

#[tokio::test]
async fn the_free_plan_is_held_to_one_project_and_names_are_checked() -> io::Result<()> {
    let server = TestServer::spawn_on_disk("cli-projects").await?;
    let cli = Cli::new(&server);
    let account = cli.signup("cli-projects").await;

    let listed = cli.run(&["--json", "projects", "list"]).await;
    listed.assert_ok();
    assert_eq!(listed.json()["projects"][0]["name"], json!(account.project));

    // The name isn't the one the website would give.
    let second_name = unique_slug("second");
    let second = cli
        .run(&[
            "--json",
            "projects",
            "create",
            "--website",
            &format!("https://{}.example", unique_slug("site")),
            "--project-name",
            &second_name,
        ])
        .await;
    // Without the billing module, signups get a complimentary plan with no such limit.
    if cfg!(feature = "billing") {
        second.assert_exit(4);
    } else {
        second.assert_ok();
        assert_eq!(second.json()["name"], json!(second_name));
    }

    cli.run(&["--project", &account.project, "tables"])
        .await
        .assert_ok();
    cli.run(&["--project", "no_such_project", "tables"])
        .await
        .assert_exit(2);

    delete_accounts(&server, &[&account.account_name]).await;
    Ok(())
}

#[tokio::test]
async fn verify_reports_what_it_could_establish() -> io::Result<()> {
    let server = TestServer::spawn_on_disk("cli-verify").await?;
    let cli = Cli::new(&server);
    let account = cli.signup("cli-verify").await;

    // The server refuses to fetch a private address, so nothing can be established.
    let local = format!("{}/", cli.api_url);
    let refused = cli
        .run(&["--json", "install", "verify", "--url", &local])
        .await;
    refused.assert_exit(7);
    assert_eq!(refused.json()["state"], json!("page_unreachable"));

    // A visit that came before the check proves it, whatever the HTML check could see,
    // both while the server still holds it in memory and after it's saved.
    post_visit(&server, &account.project, "/pricing", 1).await;
    let args = [
        "--json",
        "install",
        "verify",
        "--url",
        "https://example.com/",
        "--require-visit",
    ];
    let queued = cli.run(&args).await;
    queued.assert_ok();
    assert_eq!(queued.json()["state"], json!("receiving_data"));
    assert_eq!(queued.json()["visit"]["path"], json!("/pricing"));

    server.flush().await;
    let saved = cli.run(&args).await;
    saved.assert_ok();
    assert_eq!(saved.json()["state"], json!("receiving_data"));

    // The same lookup, asked for other hosts.
    let project_id = cli.run(&["--json", "projects", "list"]).await.json()["projects"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let token = stored_token(&cli);
    let ingest = |hostname: &'static str| {
        let path = revscope_api::paths::troubleshooting_ingest(
            &project_id,
            &revscope_api::troubleshooting::TroubleshootingIngestQuery {
                since_ms: 0,
                hostname: Some(hostname.to_string()),
            },
        );
        let token = token.clone();
        let server = &server;
        async move {
            let answer = server
                .get_query(&path, &[("Authorization", &format!("Bearer {token}"))])
                .await
                .expect("poll ingest");
            assert_eq!(answer.status_code(), 200, "{}", answer.body_str());
            answer.json::<Value>()
        }
    };
    let found = ingest("www.example.com").await;
    assert_eq!(found["detected"], json!(true), "{found}");
    assert_eq!(found["path"], json!("/pricing"));
    // A visit to another site on the same project doesn't vouch for this one.
    assert_eq!(
        ingest("staging.example.com").await["detected"],
        json!(false)
    );

    delete_accounts(&server, &[&account.account_name]).await;
    Ok(())
}

#[tokio::test]
async fn google_analytics_imports_reach_the_server() -> io::Result<()> {
    let server = TestServer::spawn_on_disk("cli-ga").await?;
    let cli = Cli::new(&server);
    let account = cli.signup("cli-ga").await;

    let listed = cli.run(&["--json", "ga-import", "list"]).await;
    listed.assert_ok();
    let listed = listed.json();
    assert_eq!(listed["imports"], json!([]));
    assert!(listed["latest_allowed_date"].is_string(), "{listed}");

    // These reach the real handlers, and fail for the reasons their own inputs give.
    let bad_key = cli
        .run_with_stdin(
            &["ga-import", "connect", "--service-account-file", "-"],
            "not a key",
        )
        .await;
    bad_key.assert_exit(2);
    cli.run(&[
        "ga-import",
        "connect",
        "--service-account-file",
        "/no/such/file",
    ])
    .await
    .assert_exit(2);
    let bad_dates = cli
        .run(&[
            "ga-import",
            "start",
            "--connection",
            "00000000-0000-7000-8000-000000000000",
            "--property",
            "123",
            "--start",
            "not-a-date",
            "--end",
            "2026-01-01",
        ])
        .await;
    bad_dates.assert_exit(2);
    assert!(
        bad_dates.stderr.contains("YYYY-MM-DD"),
        "{}",
        bad_dates.stderr
    );
    cli.run(&["ga-import", "cancel", "999999"])
        .await
        .assert_exit(6);
    cli.run(&["ga-import", "delete", "999999"])
        .await
        .assert_exit(5);

    delete_accounts(&server, &[&account.account_name]).await;
    Ok(())
}

#[tokio::test]
async fn events_waiting_for_a_flush_are_counted_before_they_are_saved() -> io::Result<()> {
    let server = TestServer::spawn_on_disk("cli-queued").await?;
    let cli = Cli::new(&server);
    let account = cli.signup("cli-queued").await;

    post_visit(&server, &account.project, "/", 1).await;
    let status = cli.run(&["--json", "install", "status"]).await;
    status.assert_ok();
    let status = status.json();
    assert_eq!(status["events_queued"], json!(1), "{status}");
    assert_eq!(status["events_last_month"], json!(0), "{status}");
    // Not in the database yet, which is why it needs saying.
    let rows = cli
        .run(&["--json", "query", "SELECT count(*) FROM pageviews"])
        .await;
    assert_eq!(rows.json()["rows"], json!([[0]]));

    server.flush().await;
    let status = cli.run(&["--json", "install", "status"]).await.json();
    assert_eq!(status["events_queued"], json!(0), "{status}");
    assert_eq!(status["events_last_month"], json!(1), "{status}");
    let rows = cli
        .run(&["--json", "query", "SELECT count(*) FROM pageviews"])
        .await;
    assert_eq!(rows.json()["rows"], json!([[1]]));

    delete_accounts(&server, &[&account.account_name]).await;
    Ok(())
}

/// Starts `revscope login`, plays the browser and the consent page against the server,
/// and returns what the CLI did.
async fn browser_login(cli: &Cli, cookie: &str, server: &TestServer, approve: bool) -> Run {
    let mut child = Command::new(cli_binary())
        .args(["--json", "login", "--no-browser", "--timeout", "30"])
        .env("REVSCOPE_API_URL", &cli.api_url)
        .env("REVSCOPE_CONFIG_DIR", &cli.config_dir)
        .env_remove("REVSCOPE_TOKEN")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the CLI");
    let stderr = child.stderr.take().expect("stderr");
    let url = tokio::task::spawn_blocking(move || {
        BufReader::new(stderr)
            .lines()
            .find_map(|line| {
                let line = line.expect("read stderr");
                line.contains("/oauth/authorize?")
                    .then(|| line.trim().to_string())
            })
            .expect("the CLI printed a sign-in URL")
    })
    .await
    .expect("join");
    let query = url.split_once('?').expect("a query").1;
    let parameters: BTreeMap<String, String> =
        serde_urlencoded::from_str(query).expect("the query");
    assert_eq!(parameters["scope"], "cli");

    let answered = server
        .post_json(
            "/v2/session/mcp/authorize",
            &json!({
                "client_id": parameters["client_id"],
                "redirect_uri": parameters["redirect_uri"],
                "state": parameters["state"],
                "code_challenge": parameters["code_challenge"],
                "code_challenge_method": "S256",
                "scope": "cli",
                "approve": approve,
            })
            .to_string(),
            &[("Cookie", cookie)],
        )
        .await
        .expect("answer the consent page");
    assert_eq!(answered.status_code(), 200, "{}", answered.body_str());
    let redirect = answered.json::<Value>()["redirect_url"]
        .as_str()
        .expect("a redirect")
        .to_string();

    // The CLI exchanges the code with the server, which runs on this runtime.
    tokio::task::spawn_blocking(move || {
        let rest = redirect.strip_prefix("http://").expect("loopback http");
        let (authority, path) = rest.split_once('/').expect("a path");
        let mut browser = TcpStream::connect(authority).expect("reach the CLI");
        write!(browser, "GET /{path} HTTP/1.1\r\nHost: {authority}\r\n\r\n").expect("send");
        let mut page = String::new();
        browser.read_to_string(&mut page).expect("read the page");
        let output = child.wait_with_output().expect("wait for the CLI");
        Run {
            code: output.status.code().expect("exit status"),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::new(),
        }
    })
    .await
    .expect("join")
}

#[tokio::test]
async fn a_browser_login_gets_a_token_and_a_denial_does_not() -> io::Result<()> {
    let server = TestServer::spawn_on_disk("cli-browser").await?;
    let signup = NewSignup::new("clibrowser");
    let response = server.post_json(SIGNUP_PATH, &signup.body(), &[]).await?;
    assert_eq!(response.status_code(), 200, "{}", response.body_str());
    let cookie = response
        .header("set-cookie")
        .expect("a session cookie")
        .split(';')
        .next()
        .expect("the cookie")
        .to_string();

    let cli = Cli::new(&server);
    let denied = browser_login(&cli, &cookie, &server, false).await;
    denied.assert_exit(3);
    assert!(!cli.credentials_file().exists(), "a denial stores nothing");

    let approved = browser_login(&cli, &cookie, &server, true).await;
    approved.assert_ok();
    assert_eq!(approved.json()["email"], json!(signup.email));
    assert!(stored_token(&cli).starts_with("rsc_"));
    let me = cli.run(&["--json", "whoami"]).await;
    me.assert_ok();
    assert_eq!(me.json()["email"], json!(signup.email));

    // The scope can't be asked for by a client that redirects off this machine.
    let remote = server
        .post_json(
            "/v2/public/mcp/register",
            &json!({ "client_name": "Remote", "redirect_uris": ["https://evil.example/cb"] })
                .to_string(),
            &[],
        )
        .await?;
    let client_id = remote.json::<Value>()["client_id"]
        .as_str()
        .expect("a client id")
        .to_string();
    let refused = server
        .post_json(
            "/v2/session/mcp/authorize",
            &json!({
                "client_id": client_id,
                "redirect_uri": "https://evil.example/cb",
                "code_challenge": "a".repeat(43),
                "code_challenge_method": "S256",
                "scope": "cli",
                "approve": true,
            })
            .to_string(),
            &[("Cookie", &cookie)],
        )
        .await?;
    assert_eq!(refused.status_code(), 400, "{}", refused.body_str());

    delete_accounts(&server, &[&signup.account_name]).await;
    Ok(())
}

async fn post_visit(server: &TestServer, project: &str, path: &str, page: u8) {
    let mut hit = TrackHit::pageview(project, path, &page_id(page));
    hit.time = Some(
        i64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap(),
    );
    let posted = server
        .post_ingest_for(project, &hit.to_form(), &[("User-Agent", CHROME_UA)])
        .await
        .expect("post a hit");
    assert_eq!(posted.status_code(), 200, "{}", posted.body_str());
}

fn stored_token(cli: &Cli) -> String {
    let stored: Value =
        serde_json::from_slice(&std::fs::read(cli.credentials_file()).expect("credentials"))
            .expect("credentials JSON");
    stored["tokens"][&cli.api_url]
        .as_str()
        .expect("a token")
        .to_string()
}

#[tokio::test]
async fn logout_keeps_the_credential_when_the_server_cannot_revoke_it() -> io::Result<()> {
    let server = TestServer::spawn_on_disk("cli-logout-down").await?;
    let mut cli = Cli::new(&server);
    let account = cli.signup("cli-logout-down").await;
    let token = stored_token(&cli);
    let live_url = cli.api_url.clone();

    // The file is keyed by URL, so this copies the credential to a dead server's URL.
    cli.api_url = "http://127.0.0.1:9".to_string();
    let stored = json!({ "tokens": { &cli.api_url: &token, &live_url: &token } });
    std::fs::write(cli.credentials_file(), stored.to_string())?;
    let down = cli.run(&["--json", "logout"]).await;
    down.assert_exit(75);
    assert_eq!(down.json()["error"]["exit_code"], json!(75));
    assert_eq!(stored_token(&cli), token, "it can still be retried");

    cli.api_url = live_url;
    cli.run(&["logout"]).await.assert_ok();
    cli.run(&["whoami"]).await.assert_exit(3);

    delete_accounts(&server, &[&account.account_name]).await;
    Ok(())
}

#[tokio::test]
async fn a_new_credential_is_tried_without_waiting_out_the_sync_throttle() -> io::Result<()> {
    let server = TestServer::spawn_on_disk("cli-throttle").await?;
    let cli = Cli::new(&server);
    let account = cli.signup("cli-throttle").await;
    cli.run(&[
        "integrations",
        "connect",
        "stripe",
        "--secret-key",
        "sk_test_one",
    ])
    .await
    .assert_ok();

    let throttled = |project: String| {
        let server = &server;
        async move {
            let conn = revscope::db::open_main_database_write(server.context.clone())
                .await
                .expect("the main database");
            conn.execute(
                "UPDATE integrations
                 SET last_sync_completed_at = NOW(), last_full_sync_success_at = NOW()
                 WHERE project_id = (SELECT id FROM projects WHERE name = $1)",
                &[&project],
            )
            .await
            .expect("mark as just synced");
        }
    };
    let is_throttled = |project: String| {
        let server = &server;
        async move {
            let conn = common::signup::main_db(server).await;
            conn.query_one(
                "SELECT last_sync_completed_at IS NOT NULL OR last_full_sync_success_at IS NOT NULL
                 FROM integrations
                 WHERE project_id = (SELECT id FROM projects WHERE name = $1)",
                &[&project],
            )
            .await
            .expect("the integration")
            .get::<_, bool>(0)
        }
    };

    throttled(account.project.clone()).await;
    cli.run(&[
        "integrations",
        "connect",
        "stripe",
        "--secret-key",
        "sk_test_two",
    ])
    .await
    .assert_ok();
    assert!(!is_throttled(account.project.clone()).await);

    // The same key again leaves the throttle alone, though it encrypts differently, or a
    // retry loop on a bad key would hammer the provider.
    throttled(account.project.clone()).await;
    cli.run(&[
        "integrations",
        "connect",
        "stripe",
        "--secret-key",
        "sk_test_two",
    ])
    .await
    .assert_ok();
    assert!(is_throttled(account.project.clone()).await);

    // So does saving with no credentials at all, as the dashboard does for unchanged ones.
    let project_id = cli.run(&["--json", "projects", "list"]).await.json()["projects"][0]["id"]
        .as_str()
        .expect("an id")
        .to_string();
    let unchanged = server
        .post_json(
            &revscope_api::paths::settings(&project_id),
            &json!({ "integrations": [{ "integration_type": "stripe", "external_account_id": null, "api_key": null, "secret": null }] })
                .to_string(),
            &[("Authorization", &format!("Bearer {}", stored_token(&cli)))],
        )
        .await?;
    assert_eq!(unchanged.status_code(), 200, "{}", unchanged.body_str());
    assert!(is_throttled(account.project.clone()).await);

    delete_accounts(&server, &[&account.account_name]).await;
    Ok(())
}

#[tokio::test]
async fn install_status_counts_only_connected_integrations() -> io::Result<()> {
    let server = TestServer::spawn_on_disk("cli-status").await?;
    let cli = Cli::new(&server);
    let account = cli.signup("cli-status").await;
    cli.run(&[
        "integrations",
        "connect",
        "stripe",
        "--secret-key",
        "sk_test_one",
    ])
    .await
    .assert_ok();
    let status = cli.run(&["--json", "install", "status"]).await.json();
    assert_eq!(
        status["integrations"][0]["integration_type"],
        json!("stripe")
    );

    cli.run(&["integrations", "disconnect", "stripe"])
        .await
        .assert_ok();
    let status = cli.run(&["--json", "install", "status"]).await.json();
    assert_eq!(status["integrations"], json!([]));

    delete_accounts(&server, &[&account.account_name]).await;
    Ok(())
}

#[tokio::test]
async fn logging_out_an_environment_token_leaves_the_stored_one_working() -> io::Result<()> {
    let server = TestServer::spawn_on_disk("cli-logout-env").await?;
    let cli = Cli::new(&server);
    let slug = unique_slug("cli-logout-env");
    let email = format!("{slug}@example.com");
    let website = format!("https://{slug}.example");
    cli.run_with_stdin(
        &[
            "signup",
            "--name",
            "Ada",
            "--email",
            &email,
            "--website",
            &website,
            "--password-stdin",
        ],
        &format!("{PASSWORD}\n"),
    )
    .await
    .assert_ok();

    // A second token for the same user, from another machine.
    let mut other = Cli::new(&server);
    other
        .run_with_stdin(
            &["login", "--email", &email, "--password-stdin"],
            &format!("{PASSWORD}\n"),
        )
        .await
        .assert_ok();
    let second = stored_token(&other);
    other.token = None;

    // Logging out with it in the environment revokes it, and nothing else.
    let mut with_env = Cli::new(&server);
    with_env.config_dir = cli.config_dir.clone();
    with_env.token = Some(second.clone());
    let logged_out = with_env.run(&["logout"]).await;
    with_env.config_dir = std::env::temp_dir().join("revscope-cli-e2e-unused");
    logged_out.assert_ok();
    assert!(
        logged_out.stdout.contains("REVSCOPE_TOKEN"),
        "{}",
        logged_out.stdout
    );
    cli.run(&["whoami"]).await.assert_ok();
    other.run(&["whoami"]).await.assert_exit(3);

    delete_accounts(&server, &[&format!("{slug}.example")]).await;
    Ok(())
}

#[test]
fn failures_of_the_command_line_itself_keep_the_contract() {
    let run = |args: &[&str], envs: &[(&str, &str)]| {
        Command::new(cli_binary())
            .args(args)
            .env("REVSCOPE_API_URL", "http://127.0.0.1:9")
            .env(
                "REVSCOPE_CONFIG_DIR",
                std::env::temp_dir().join("revscope-cli-e2e-none"),
            )
            .envs(envs.iter().copied())
            .output()
            .expect("run the CLI")
    };

    let missing_sql = run(&["--json", "query"], &[]);
    assert_eq!(missing_sql.status.code(), Some(2));
    let failure: Value = serde_json::from_slice(&missing_sql.stdout).expect("JSON on stdout");
    assert_eq!(failure["error"]["exit_code"], json!(2));

    let plain = run(&["query"], &[]);
    assert_eq!(plain.status.code(), Some(2));
    assert!(plain.stdout.is_empty());

    assert_eq!(run(&["--help"], &[]).status.code(), Some(0));
    assert_eq!(run(&["--version"], &[]).status.code(), Some(0));

    // Not UTF-8, which a naive look at the arguments panics on.
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let bad = std::ffi::OsStr::from_bytes(b"\xff");
        let output = Command::new(cli_binary())
            .arg("query")
            .arg(bad)
            .arg("--json")
            .env("REVSCOPE_API_URL", "http://127.0.0.1:9")
            .output()
            .expect("run the CLI");
        assert_eq!(output.status.code(), Some(2));
    }

    // Nothing but a token from the environment needs no home directory.
    let no_home = Command::new(cli_binary())
        .args(["whoami"])
        .env_clear()
        .env("REVSCOPE_TOKEN", "rsc_x")
        .env("REVSCOPE_API_URL", "http://127.0.0.1:9")
        .output()
        .expect("run the CLI");
    assert_eq!(
        no_home.status.code(),
        Some(75),
        "it should reach for the server"
    );

    // A bad credential is a mistake to fix, not something to retry.
    let bad_token = run(&["whoami"], &[("REVSCOPE_TOKEN", "abc\ndef")]);
    assert_eq!(bad_token.status.code(), Some(2));
}

#[cfg(target_os = "linux")]
#[test]
fn a_full_disk_is_not_reported_as_success() {
    let output = Command::new(cli_binary())
        .args(["integrations", "types"])
        .stdout(
            std::fs::OpenOptions::new()
                .write(true)
                .open("/dev/full")
                .expect("/dev/full"),
        )
        .output()
        .expect("run the CLI");
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn a_closed_stdout_is_not_a_panic() {
    let mut child = Command::new(cli_binary())
        .args(["integrations", "types"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the CLI");
    drop(child.stdout.take());
    let output = child.wait_with_output().expect("wait");
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn the_catalog_command_works_offline_with_a_corrupt_credentials_file() {
    let config_dir =
        std::env::temp_dir().join(format!("revscope-cli-e2e-offline-{}", std::process::id()));
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(config_dir.join("credentials.json"), "not json").unwrap();
    let output = Command::new(cli_binary())
        .args(["--json", "integrations", "types"])
        .env("REVSCOPE_CONFIG_DIR", &config_dir)
        .env("REVSCOPE_API_URL", "http://127.0.0.1:9")
        .output()
        .expect("run the CLI");
    assert!(output.status.success());
    let catalog: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        catalog["integrations"].as_array().unwrap().len(),
        revscope_api::integrations::INTEGRATIONS.len()
    );
    let _ = std::fs::remove_dir_all(&config_dir);
}
