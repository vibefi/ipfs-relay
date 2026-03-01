//! E2E tests for the IPFS relay upload path.
//!
//! These tests require a running Kubo IPFS node (default: http://127.0.0.1:5001).
//! Override with: VIBEFI_RELAY__IPFS__KUBO_API_URL=http://host:port
//!
//! Run:   cargo test --test upload_e2e -- --ignored
//! Start: docker run -d --name kubo -p 5001:5001 ipfs/kubo:v0.32.1

use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::http::{header, HeaderValue, StatusCode};
use axum::routing::{get, post};
use axum::Router;
use axum_test::multipart::{MultipartForm, Part};
use axum_test::TestServer;
use serde_json::json;

use ipfs_relay::config::*;
use ipfs_relay::ipfs::KuboClient;
use ipfs_relay::middleware::auth::auth_middleware;
use ipfs_relay::pinning::PinningService;
use ipfs_relay::routes::uploads;
use ipfs_relay::storage::db::Database;
use ipfs_relay::AppState;

// ── Test infrastructure ─────────────────────────────────────────────────────

async fn test_server() -> TestServer {
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

    // Router without GovernorLayer to avoid rate-limit flakiness in tests
    let app = Router::new()
        .route("/v1/uploads", post(uploads::create_upload))
        .route("/v1/uploads/{upload_id}", get(uploads::get_upload))
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024 + 64 * 1024))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .with_state(state);

    TestServer::new(app)
}

// ── Bundle builders ─────────────────────────────────────────────────────────

fn valid_bundle() -> MultipartForm {
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

    MultipartForm::new()
        .add_part(
            "file",
            Part::bytes(serde_json::to_vec(&manifest).unwrap()).file_name("manifest.json"),
        )
        .add_part("file", Part::bytes(b"{}".to_vec()).file_name("vibefi.json"))
        .add_part(
            "file",
            Part::bytes(index_html.to_vec()).file_name("index.html"),
        )
}

fn multi_file_bundle() -> MultipartForm {
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

    MultipartForm::new()
        .add_part(
            "file",
            Part::bytes(serde_json::to_vec(&manifest).unwrap()).file_name("manifest.json"),
        )
        .add_part("file", Part::bytes(b"{}".to_vec()).file_name("vibefi.json"))
        .add_part(
            "file",
            Part::bytes(index_html.to_vec()).file_name("index.html"),
        )
        .add_part(
            "file",
            Part::bytes(app_js.to_vec()).file_name("src/app.js"),
        )
        .add_part(
            "file",
            Part::bytes(style_css.to_vec()).file_name("styles/style.css"),
        )
}

// ── CID verification ────────────────────────────────────────────────────────

async fn verify_cid_on_kubo(cid: &str) {
    let kubo_url = std::env::var("VIBEFI_RELAY__IPFS__KUBO_API_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:5001".to_string());
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
    let kubo_url = std::env::var("VIBEFI_RELAY__IPFS__KUBO_API_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:5001".to_string());
    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{kubo_url}/api/v0/cat?arg={root_cid}/{path}"
        ))
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

// ── Happy path tests ────────────────────────────────────────────────────────

/// 1. Minimal valid bundle → 201, correct response shape, CID resolvable on Kubo
#[tokio::test]
#[ignore]
async fn upload_valid_bundle_returns_201() {
    let server = test_server().await;
    let response = server.post("/v1/uploads").multipart(valid_bundle()).await;

    assert_eq!(response.status_code(), StatusCode::CREATED);

    let body: serde_json::Value = response.json();
    assert!(body["uploadId"].as_str().unwrap().starts_with("upl_"));
    assert!(!body["rootCid"].as_str().unwrap().is_empty());
    assert!(body["bytes"].as_u64().unwrap() > 0);
    assert_eq!(body["fileCount"].as_u64().unwrap(), 3);
    assert_eq!(body["validation"]["isVibeFiPackage"].as_bool(), Some(true));
    assert_eq!(body["pinning"]["local"].as_str(), Some("pinned"));

    // Verify CID exists on Kubo
    let cid = body["rootCid"].as_str().unwrap();
    verify_cid_on_kubo(cid).await;
}

/// 2. Bundle with subdirectories → 201
#[tokio::test]
#[ignore]
async fn upload_multi_file_bundle_returns_201() {
    let server = test_server().await;
    let response = server
        .post("/v1/uploads")
        .multipart(multi_file_bundle())
        .await;

    assert_eq!(response.status_code(), StatusCode::CREATED);

    let body: serde_json::Value = response.json();
    assert!(body["uploadId"].as_str().unwrap().starts_with("upl_"));
    assert!(!body["rootCid"].as_str().unwrap().is_empty());
    assert_eq!(body["fileCount"].as_u64().unwrap(), 5);
}

/// 3. POST then GET status roundtrip
#[tokio::test]
#[ignore]
async fn upload_then_get_status() {
    let server = test_server().await;

    let post_response = server.post("/v1/uploads").multipart(valid_bundle()).await;
    assert_eq!(post_response.status_code(), StatusCode::CREATED);

    let post_body: serde_json::Value = post_response.json();
    let upload_id = post_body["uploadId"].as_str().unwrap();

    let get_response = server
        .get(&format!("/v1/uploads/{upload_id}"))
        .await;
    assert_eq!(get_response.status_code(), StatusCode::OK);

    let get_body: serde_json::Value = get_response.json();
    assert_eq!(get_body["uploadId"].as_str().unwrap(), upload_id);
    assert_eq!(
        get_body["rootCid"].as_str().unwrap(),
        post_body["rootCid"].as_str().unwrap()
    );
    // Status should be either pending (no remote pinning configured) or completed
    let status = get_body["status"].as_str().unwrap();
    assert!(
        ["pending", "completed"].contains(&status),
        "unexpected status: {status}"
    );
}

/// 4. Upload with valid API key → 201
#[tokio::test]
#[ignore]
async fn upload_with_api_key_succeeds() {
    let server = test_server().await;
    let response = server
        .post("/v1/uploads")
        .add_header(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer test-key"),
        )
        .multipart(valid_bundle())
        .await;

    assert_eq!(response.status_code(), StatusCode::CREATED);
}

/// 5. Same content uploaded twice produces the same CID
#[tokio::test]
#[ignore]
async fn deterministic_cid_for_same_content() {
    let server = test_server().await;

    let response1 = server.post("/v1/uploads").multipart(valid_bundle()).await;
    assert_eq!(response1.status_code(), StatusCode::CREATED);
    let body1: serde_json::Value = response1.json();

    let response2 = server.post("/v1/uploads").multipart(valid_bundle()).await;
    assert_eq!(response2.status_code(), StatusCode::CREATED);
    let body2: serde_json::Value = response2.json();

    assert_eq!(
        body1["rootCid"].as_str().unwrap(),
        body2["rootCid"].as_str().unwrap(),
        "same content should produce the same CID"
    );
}

/// 6. Upload then download each file from IPFS and verify content matches
#[tokio::test]
#[ignore]
async fn upload_then_download_from_ipfs() {
    let server = test_server().await;

    let index_html = b"<html><body>Hello VibeFi</body></html>";
    let app_js = b"console.log('app');";
    let style_css = b"body { margin: 0; }";

    let response = server
        .post("/v1/uploads")
        .multipart(multi_file_bundle())
        .await;
    assert_eq!(response.status_code(), StatusCode::CREATED);

    let body: serde_json::Value = response.json();
    let root_cid = body["rootCid"].as_str().unwrap();

    // Fetch each file back from Kubo and verify content matches
    assert_eq!(
        fetch_file_from_kubo(root_cid, "index.html").await,
        index_html,
        "index.html content mismatch"
    );
    assert_eq!(
        fetch_file_from_kubo(root_cid, "src/app.js").await,
        app_js,
        "src/app.js content mismatch"
    );
    assert_eq!(
        fetch_file_from_kubo(root_cid, "styles/style.css").await,
        style_css,
        "styles/style.css content mismatch"
    );
}

// ── Error case tests ────────────────────────────────────────────────────────

/// 6. Missing manifest.json → 400 INVALID_PACKAGE
#[tokio::test]
#[ignore]
async fn upload_missing_manifest_returns_400() {
    let server = test_server().await;

    let form = MultipartForm::new()
        .add_part("file", Part::bytes(b"{}".to_vec()).file_name("vibefi.json"))
        .add_part(
            "file",
            Part::bytes(b"<html></html>".to_vec()).file_name("index.html"),
        );

    let response = server.post("/v1/uploads").multipart(form).await;
    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);

    let body: serde_json::Value = response.json();
    assert_eq!(body["error"]["code"].as_str(), Some("INVALID_PACKAGE"));
}

/// 7. Has manifest but no vibefi.json → 400 INVALID_PACKAGE
#[tokio::test]
#[ignore]
async fn upload_missing_vibefi_json_returns_400() {
    let server = test_server().await;

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

    let form = MultipartForm::new()
        .add_part(
            "file",
            Part::bytes(serde_json::to_vec(&manifest).unwrap()).file_name("manifest.json"),
        )
        .add_part(
            "file",
            Part::bytes(index_html.to_vec()).file_name("index.html"),
        );

    let response = server.post("/v1/uploads").multipart(form).await;
    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);

    let body: serde_json::Value = response.json();
    assert_eq!(body["error"]["code"].as_str(), Some("INVALID_PACKAGE"));
}

/// 8. Path traversal in filename → 400 INVALID_PACKAGE
#[tokio::test]
#[ignore]
async fn upload_path_traversal_returns_400() {
    let server = test_server().await;

    let form = MultipartForm::new()
        .add_part("file", Part::bytes(b"{}".to_vec()).file_name("manifest.json"))
        .add_part("file", Part::bytes(b"{}".to_vec()).file_name("vibefi.json"))
        .add_part(
            "file",
            Part::bytes(b"pwned".to_vec()).file_name("../../etc/passwd"),
        );

    let response = server.post("/v1/uploads").multipart(form).await;
    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);

    let body: serde_json::Value = response.json();
    assert_eq!(body["error"]["code"].as_str(), Some("INVALID_PACKAGE"));
}

/// 9. Invalid API key → 401 UNAUTHORIZED
#[tokio::test]
#[ignore]
async fn upload_invalid_api_key_returns_401() {
    let server = test_server().await;

    let response = server
        .post("/v1/uploads")
        .add_header(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer wrong-key"),
        )
        .multipart(valid_bundle())
        .await;

    assert_eq!(response.status_code(), StatusCode::UNAUTHORIZED);

    let body: serde_json::Value = response.json();
    assert_eq!(body["error"]["code"].as_str(), Some("UNAUTHORIZED"));
}

/// 10. GET nonexistent upload → 404 NOT_FOUND
#[tokio::test]
#[ignore]
async fn get_nonexistent_upload_returns_404() {
    let server = test_server().await;

    let response = server.get("/v1/uploads/upl_nonexistent").await;
    assert_eq!(response.status_code(), StatusCode::NOT_FOUND);

    let body: serde_json::Value = response.json();
    assert_eq!(body["error"]["code"].as_str(), Some("NOT_FOUND"));
}
