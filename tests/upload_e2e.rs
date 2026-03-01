//! E2E tests for the IPFS relay upload path.
//!
//! Modes:
//! 1) Local in-process relay (default)
//!    Run: cargo test --test upload_e2e -- --ignored
//!
//! 2) Remote relay endpoint (override base URL)
//!    Run: VIBEFI_RELAY_E2E_BASE_URL=http://<SERVER_IP> cargo test --test upload_e2e -- --ignored
//!
//! Optional env vars:
//! - VIBEFI_RELAY_E2E_KUBO_API_URL=http://host:5001
//!     Required only for tests that directly verify CID/file retrieval on Kubo.
//!     If unset in remote mode, Kubo-specific checks are skipped.
//! - VIBEFI_RELAY_E2E_API_KEY=<key>
//!     API key used by `upload_with_api_key_succeeds` (default: test-key).
//! - VIBEFI_RELAY_E2E_EXPECT_API_KEY_AUTH=true|false
//!     Controls whether `upload_invalid_api_key_returns_401` should run.
//!     Default: true for local mode, false for remote mode.

use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use axum::Router;
use reqwest::header::AUTHORIZATION;
use reqwest::multipart::{Form, Part};
use reqwest::StatusCode;
use serde_json::json;

use ipfs_relay::config::*;
use ipfs_relay::ipfs::KuboClient;
use ipfs_relay::middleware::auth::auth_middleware;
use ipfs_relay::pinning::PinningService;
use ipfs_relay::routes::uploads;
use ipfs_relay::storage::db::Database;
use ipfs_relay::AppState;

#[derive(Clone)]
struct UploadFileSpec {
    path: String,
    content: Vec<u8>,
}

struct RelayClient {
    base_url: String,
    http: reqwest::Client,
    // Keep local test server alive for the duration of a test.
    _server_task: Option<tokio::task::JoinHandle<()>>,
}

struct HttpResponse {
    status: StatusCode,
    body: serde_json::Value,
}

fn remote_mode() -> bool {
    std::env::var("VIBEFI_RELAY_E2E_BASE_URL").is_ok()
}

fn bool_env(name: &str, default: bool) -> bool {
    match std::env::var(name) {
        Ok(v) => matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"),
        Err(_) => default,
    }
}

fn expect_api_key_auth_enforcement() -> bool {
    bool_env(
        "VIBEFI_RELAY_E2E_EXPECT_API_KEY_AUTH",
        !remote_mode(),
    )
}

fn success_api_key() -> String {
    std::env::var("VIBEFI_RELAY_E2E_API_KEY").unwrap_or_else(|_| "test-key".to_string())
}

fn kubo_api_url() -> Option<String> {
    if let Ok(url) = std::env::var("VIBEFI_RELAY_E2E_KUBO_API_URL") {
        return Some(url);
    }

    if remote_mode() {
        return None;
    }

    Some(
        std::env::var("VIBEFI_RELAY__IPFS__KUBO_API_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:5001".to_string()),
    )
}

fn kubo_checks_enabled() -> bool {
    kubo_api_url().is_some()
}

async fn test_client() -> RelayClient {
    let http = reqwest::Client::new();

    if let Ok(base_url) = std::env::var("VIBEFI_RELAY_E2E_BASE_URL") {
        return RelayClient {
            base_url: base_url.trim_end_matches('/').to_string(),
            http,
            _server_task: None,
        };
    }

    let kubo_url = std::env::var("VIBEFI_RELAY__IPFS__KUBO_API_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:5001".to_string());

    let db_path = std::env::temp_dir().join(format!("ipfs_relay_test_{}.db", ulid::Ulid::new()));
    let db_url = format!("sqlite:{}", db_path.display());

    let config = AppConfig {
        server: ServerConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            request_timeout_secs: 120,
        },
        database: DatabaseConfig {
            url: db_url,
            retention_days: 90,
        },
        ipfs: IpfsConfig {
            kubo_api_url: kubo_url,
        },
        pinning: PinningConfig {
            pinata_jwt: None,
            foureverland_token: None,
        },
        limits: LimitsConfig {
            max_upload_bytes: 10 * 1024 * 1024,
            max_file_count: 1500,
            max_single_file_bytes: 5 * 1024 * 1024,
            strict_manifest: false,
        },
        rate_limit: RateLimitConfig {
            per_ip_per_hour: 30,
            per_key_per_day: 300,
        },
        auth: AuthConfig {
            api_keys: Some("test-key".to_string()),
        },
    };

    let db = Database::connect(&config.database.url)
        .await
        .expect("db connect");
    db.migrate().await.expect("db migrate");

    let ipfs_client = Arc::new(KuboClient::new(&config.ipfs.kubo_api_url));
    let pinning_svc = Arc::new(PinningService::new(
        db.clone(),
        Arc::clone(&ipfs_client),
        config.pinning.clone(),
    ));

    let state = AppState {
        config: Arc::new(config),
        db,
        ipfs: ipfs_client,
        pinning: pinning_svc,
    };

    // Router without GovernorLayer to avoid rate-limit flakiness in local tests.
    let app = Router::new()
        .route("/v1/uploads", post(uploads::create_upload))
        .route("/v1/uploads/{upload_id}", get(uploads::get_upload))
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024 + 64 * 1024))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local test listener");
    let addr = listener.local_addr().expect("read local address");

    let server_task = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .expect("local test server failed");
    });

    RelayClient {
        base_url: format!("http://{addr}"),
        http,
        _server_task: Some(server_task),
    }
}

fn multipart_form(files: &[UploadFileSpec]) -> Form {
    files.iter().fold(Form::new(), |form, f| {
        form.part(
            "file",
            Part::bytes(f.content.clone()).file_name(f.path.clone()),
        )
    })
}

async fn parse_response(resp: reqwest::Response) -> HttpResponse {
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let body = serde_json::from_str::<serde_json::Value>(&text)
        .unwrap_or_else(|_| json!({"raw": text}));
    HttpResponse { status, body }
}

impl RelayClient {
    async fn post_upload(&self, files: Vec<UploadFileSpec>, api_key: Option<&str>) -> HttpResponse {
        let mut req = self
            .http
            .post(format!("{}/v1/uploads", self.base_url))
            .header("X-Forwarded-For", "203.0.113.10")
            .multipart(multipart_form(&files));

        if let Some(key) = api_key {
            req = req.header(AUTHORIZATION, format!("Bearer {key}"));
        }

        let resp = req.send().await.expect("POST /v1/uploads request failed");
        parse_response(resp).await
    }

    async fn get_json(&self, path: &str) -> HttpResponse {
        let resp = self
            .http
            .get(format!("{}{}", self.base_url, path))
            .header("X-Forwarded-For", "203.0.113.10")
            .send()
            .await
            .expect("GET request failed");
        parse_response(resp).await
    }
}

fn valid_bundle() -> Vec<UploadFileSpec> {
    let index_html = b"<html><body>Hello VibeFi</body></html>";
    let manifest = json!({
        "name": "test-app",
        "version": "1.0.0",
        "createdAt": "2026-01-01T00:00:00Z",
        "layout": "spa",
        "entry": "index.html",
        "files": [
            { "path": "index.html", "bytes": index_html.len() }
        ]
    });

    vec![
        UploadFileSpec {
            path: "manifest.json".to_string(),
            content: serde_json::to_vec(&manifest).unwrap(),
        },
        UploadFileSpec {
            path: "vibefi.json".to_string(),
            content: b"{}".to_vec(),
        },
        UploadFileSpec {
            path: "index.html".to_string(),
            content: index_html.to_vec(),
        },
    ]
}

fn multi_file_bundle() -> Vec<UploadFileSpec> {
    let index_html = b"<html><body>Hello VibeFi</body></html>";
    let app_js = b"console.log('app');";
    let style_css = b"body { margin: 0; }";

    let manifest = json!({
        "name": "test-multi-app",
        "version": "1.0.0",
        "createdAt": "2026-01-01T00:00:00Z",
        "layout": "spa",
        "entry": "index.html",
        "files": [
            { "path": "index.html", "bytes": index_html.len() },
            { "path": "src/app.js", "bytes": app_js.len() },
            { "path": "styles/style.css", "bytes": style_css.len() }
        ]
    });

    vec![
        UploadFileSpec {
            path: "manifest.json".to_string(),
            content: serde_json::to_vec(&manifest).unwrap(),
        },
        UploadFileSpec {
            path: "vibefi.json".to_string(),
            content: b"{}".to_vec(),
        },
        UploadFileSpec {
            path: "index.html".to_string(),
            content: index_html.to_vec(),
        },
        UploadFileSpec {
            path: "src/app.js".to_string(),
            content: app_js.to_vec(),
        },
        UploadFileSpec {
            path: "styles/style.css".to_string(),
            content: style_css.to_vec(),
        },
    ]
}

async fn verify_cid_on_kubo(cid: &str) {
    let Some(kubo_url) = kubo_api_url() else {
        return;
    };

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{kubo_url}/api/v0/dag/stat?arg={cid}"))
        .send()
        .await
        .expect("kubo dag/stat request failed");

    assert!(
        resp.status().is_success(),
        "CID {cid} not found on Kubo: status {}",
        resp.status()
    );
}

async fn fetch_file_from_kubo(root_cid: &str, path: &str) -> Vec<u8> {
    let kubo_url = kubo_api_url().expect("kubo url is required for file fetch");

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{kubo_url}/api/v0/cat?arg={root_cid}/{path}"))
        .send()
        .await
        .expect("kubo cat request failed");

    assert!(
        resp.status().is_success(),
        "failed to fetch {path} from Kubo: status {}",
        resp.status()
    );

    resp.bytes().await.expect("failed to read body").to_vec()
}

#[tokio::test]
#[ignore]
async fn upload_valid_bundle_returns_201() {
    let relay = test_client().await;
    let response = relay.post_upload(valid_bundle(), None).await;

    assert_eq!(response.status, StatusCode::CREATED);

    let body = response.body;
    assert!(body["uploadId"].as_str().unwrap().starts_with("upl_"));
    assert!(!body["rootCid"].as_str().unwrap().is_empty());
    assert!(body["bytes"].as_u64().unwrap() > 0);
    assert_eq!(body["fileCount"].as_u64().unwrap(), 3);
    assert_eq!(body["validation"]["isVibeFiPackage"].as_bool(), Some(true));
    assert_eq!(body["pinning"]["local"].as_str(), Some("pinned"));

    if kubo_checks_enabled() {
        verify_cid_on_kubo(body["rootCid"].as_str().unwrap()).await;
    }
}

#[tokio::test]
#[ignore]
async fn upload_multi_file_bundle_returns_201() {
    let relay = test_client().await;
    let response = relay.post_upload(multi_file_bundle(), None).await;

    assert_eq!(response.status, StatusCode::CREATED);

    let body = response.body;
    assert!(body["uploadId"].as_str().unwrap().starts_with("upl_"));
    assert!(!body["rootCid"].as_str().unwrap().is_empty());
    assert_eq!(body["fileCount"].as_u64().unwrap(), 5);
}

#[tokio::test]
#[ignore]
async fn upload_then_get_status() {
    let relay = test_client().await;

    let post_response = relay.post_upload(valid_bundle(), None).await;
    assert_eq!(post_response.status, StatusCode::CREATED);

    let post_body = post_response.body;
    let upload_id = post_body["uploadId"].as_str().unwrap();

    let get_response = relay.get_json(&format!("/v1/uploads/{upload_id}")).await;
    assert_eq!(get_response.status, StatusCode::OK);

    let get_body = get_response.body;
    assert_eq!(get_body["uploadId"].as_str().unwrap(), upload_id);
    assert_eq!(
        get_body["rootCid"].as_str().unwrap(),
        post_body["rootCid"].as_str().unwrap()
    );

    let status = get_body["status"].as_str().unwrap();
    assert!(
        ["pending", "completed", "partial"].contains(&status),
        "unexpected status: {status}"
    );
}

#[tokio::test]
#[ignore]
async fn upload_with_api_key_succeeds() {
    let relay = test_client().await;
    let key = success_api_key();

    let response = relay.post_upload(valid_bundle(), Some(&key)).await;
    assert_eq!(response.status, StatusCode::CREATED);
}

#[tokio::test]
#[ignore]
async fn deterministic_cid_for_same_content() {
    let relay = test_client().await;

    let response1 = relay.post_upload(valid_bundle(), None).await;
    assert_eq!(response1.status, StatusCode::CREATED);

    let response2 = relay.post_upload(valid_bundle(), None).await;
    assert_eq!(response2.status, StatusCode::CREATED);

    assert_eq!(
        response1.body["rootCid"].as_str().unwrap(),
        response2.body["rootCid"].as_str().unwrap(),
        "same content should produce the same CID"
    );
}

#[tokio::test]
#[ignore]
async fn upload_then_download_from_ipfs() {
    if !kubo_checks_enabled() {
        eprintln!(
            "Skipping Kubo content verification: set VIBEFI_RELAY_E2E_KUBO_API_URL to enable"
        );
        return;
    }

    let relay = test_client().await;

    let index_html = b"<html><body>Hello VibeFi</body></html>";
    let app_js = b"console.log('app');";
    let style_css = b"body { margin: 0; }";

    let response = relay.post_upload(multi_file_bundle(), None).await;
    assert_eq!(response.status, StatusCode::CREATED);

    let root_cid = response.body["rootCid"].as_str().unwrap().to_string();

    assert_eq!(
        fetch_file_from_kubo(&root_cid, "index.html").await,
        index_html,
        "index.html content mismatch"
    );
    assert_eq!(
        fetch_file_from_kubo(&root_cid, "src/app.js").await,
        app_js,
        "src/app.js content mismatch"
    );
    assert_eq!(
        fetch_file_from_kubo(&root_cid, "styles/style.css").await,
        style_css,
        "styles/style.css content mismatch"
    );
}

#[tokio::test]
#[ignore]
async fn upload_missing_manifest_returns_400() {
    let relay = test_client().await;

    let form = vec![
        UploadFileSpec {
            path: "vibefi.json".to_string(),
            content: b"{}".to_vec(),
        },
        UploadFileSpec {
            path: "index.html".to_string(),
            content: b"<html></html>".to_vec(),
        },
    ];

    let response = relay.post_upload(form, None).await;
    assert_eq!(response.status, StatusCode::BAD_REQUEST);
    assert_eq!(response.body["error"]["code"].as_str(), Some("INVALID_PACKAGE"));
}

#[tokio::test]
#[ignore]
async fn upload_missing_vibefi_json_returns_400() {
    let relay = test_client().await;

    let index_html = b"<html></html>";
    let manifest = json!({
        "name": "test-app",
        "version": "1.0.0",
        "createdAt": "2026-01-01T00:00:00Z",
        "layout": "spa",
        "entry": "index.html",
        "files": [
            { "path": "index.html", "bytes": index_html.len() }
        ]
    });

    let form = vec![
        UploadFileSpec {
            path: "manifest.json".to_string(),
            content: serde_json::to_vec(&manifest).unwrap(),
        },
        UploadFileSpec {
            path: "index.html".to_string(),
            content: index_html.to_vec(),
        },
    ];

    let response = relay.post_upload(form, None).await;
    assert_eq!(response.status, StatusCode::BAD_REQUEST);
    assert_eq!(response.body["error"]["code"].as_str(), Some("INVALID_PACKAGE"));
}

#[tokio::test]
#[ignore]
async fn upload_path_traversal_returns_400() {
    let relay = test_client().await;

    let form = vec![
        UploadFileSpec {
            path: "manifest.json".to_string(),
            content: b"{}".to_vec(),
        },
        UploadFileSpec {
            path: "vibefi.json".to_string(),
            content: b"{}".to_vec(),
        },
        UploadFileSpec {
            path: "../../etc/passwd".to_string(),
            content: b"pwned".to_vec(),
        },
    ];

    let response = relay.post_upload(form, None).await;
    assert_eq!(response.status, StatusCode::BAD_REQUEST);
    assert_eq!(response.body["error"]["code"].as_str(), Some("INVALID_PACKAGE"));
}

#[tokio::test]
#[ignore]
async fn upload_invalid_api_key_returns_401() {
    if !expect_api_key_auth_enforcement() {
        eprintln!(
            "Skipping invalid-key auth check: set VIBEFI_RELAY_E2E_EXPECT_API_KEY_AUTH=true to enforce"
        );
        return;
    }

    let relay = test_client().await;

    let response = relay.post_upload(valid_bundle(), Some("wrong-key")).await;
    assert_eq!(response.status, StatusCode::UNAUTHORIZED);
    assert_eq!(response.body["error"]["code"].as_str(), Some("UNAUTHORIZED"));
}

#[tokio::test]
#[ignore]
async fn get_nonexistent_upload_returns_404() {
    let relay = test_client().await;

    let response = relay.get_json("/v1/uploads/upl_nonexistent").await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    assert_eq!(response.body["error"]["code"].as_str(), Some("NOT_FOUND"));
}
