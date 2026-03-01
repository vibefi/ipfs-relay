use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum_prometheus::PrometheusMetricLayer;
use tower_http::{
    compression::CompressionLayer, cors::CorsLayer, normalize_path::NormalizePathLayer,
    timeout::TimeoutLayer, trace::TraceLayer,
};
use tracing::info;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

use ipfs_relay::AppState;
use ipfs_relay::config::AppConfig;
use ipfs_relay::ipfs;
use ipfs_relay::middleware::tracing::{make_span, on_response};
use ipfs_relay::pinning;
use ipfs_relay::routes;
use ipfs_relay::storage::db::Database;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ── Tracing ──────────────────────────────────────────────────────────────
    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .json()
                .with_current_span(true)
                .with_span_list(false),
        )
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ipfs_relay=info,tower_http=info".into()),
        )
        .init();

    // ── Config ───────────────────────────────────────────────────────────────
    let config = Arc::new(AppConfig::load()?);
    info!(host = %config.server.host, port = config.server.port, "starting ipfs-relay");

    // ── Database ─────────────────────────────────────────────────────────────
    let db = Database::connect(&config.database.url).await?;
    db.migrate().await?;

    // ── IPFS client ──────────────────────────────────────────────────────────
    let ipfs_client = Arc::new(ipfs::KuboClient::new(&config.ipfs.kubo_api_url));

    // ── Pinning service ──────────────────────────────────────────────────────
    let pinning_svc = Arc::new(pinning::PinningService::new(
        db.clone(),
        Arc::clone(&ipfs_client),
        config.pinning.clone(),
    ));
    pinning_svc.clone().start_worker();

    // ── App state ────────────────────────────────────────────────────────────
    let state = AppState {
        config: Arc::clone(&config),
        db,
        ipfs: ipfs_client,
        pinning: pinning_svc,
    };

    // ── Metrics ──────────────────────────────────────────────────────────────
    let (prometheus_layer, metrics_handle) = PrometheusMetricLayer::pair();

    // ── Router ───────────────────────────────────────────────────────────────
    let app = Router::new()
        .merge(routes::api_router(state.clone()))
        .merge(routes::meta_router(metrics_handle))
        .layer(NormalizePathLayer::trim_trailing_slash())
        .layer(TimeoutLayer::new(std::time::Duration::from_secs(
            config.server.request_timeout_secs,
        )))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(make_span)
                .on_response(on_response),
        )
        .layer(prometheus_layer)
        .layer(CompressionLayer::new())
        .layer(CorsLayer::permissive());

    // ── Listen ───────────────────────────────────────────────────────────────
    let addr: SocketAddr = format!("{}:{}", config.server.host, config.server.port).parse()?;
    info!(%addr, "listening");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
