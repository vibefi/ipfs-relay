pub mod health;
pub mod uploads;

use std::sync::Arc;
use std::time::Duration;

use axum::{
    body::Body,
    extract::DefaultBodyLimit,
    middleware,
    routing::{get, post},
    Router,
};
use axum_prometheus::metrics_exporter_prometheus::PrometheusHandle;
use tower_governor::{governor::GovernorConfigBuilder, key_extractor::SmartIpKeyExtractor, GovernorLayer};

use crate::AppState;
use crate::middleware::auth::auth_middleware;

/// `/v1/*` routes with auth middleware, rate limiting, and body size cap.
pub fn api_router(state: AppState) -> Router {
    // IP-based token bucket: 30 uploads/hour (spec § 7), burst of 5.
    // 30/hour = 1 token per 120 seconds.
    let governor_conf = Arc::new(
        GovernorConfigBuilder::default()
            .seconds_per_request(120)
            .burst_size(5)
            .key_extractor(SmartIpKeyExtractor)
            .finish()
            .unwrap(),
    );

    // Periodically evict expired rate-limit entries to keep memory bounded.
    let limiter = Arc::clone(governor_conf.limiter());
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            limiter.retain_recent();
        }
    });

    Router::new()
        .route("/v1/uploads", post(uploads::create_upload))
        .route("/v1/uploads/{upload_id}", get(uploads::get_upload))
        // Enforce body size limit before multipart is buffered into memory.
        // 10 MiB payload cap + headroom for multipart framing overhead.
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024 + 64 * 1024))
        .layer(GovernorLayer { config: governor_conf })
        .layer(middleware::from_fn_with_state(state.clone(), auth_middleware))
        .with_state(state)
}

/// Meta / operational routes (no auth or rate limiting required)
pub fn meta_router(metrics: PrometheusHandle) -> Router {
    Router::new()
        .route("/health", get(health::health_check))
        .route(
            "/metrics",
            get(move || async move { metrics.render() }),
        )
}
