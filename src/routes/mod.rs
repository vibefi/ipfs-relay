pub mod health;
pub mod uploads;

use std::sync::Arc;
use std::time::Duration;

use axum::{
    Router,
    extract::DefaultBodyLimit,
    middleware,
    routing::{get, post},
};
use axum_prometheus::metrics_exporter_prometheus::PrometheusHandle;
use tower_governor::{
    GovernorLayer, governor::GovernorConfigBuilder, key_extractor::SmartIpKeyExtractor,
};

use crate::AppState;
use crate::middleware::auth::auth_middleware;

/// `/v1/*` routes with auth middleware, rate limiting, and body size cap.
pub fn api_router(state: AppState) -> Router {
    // IP-based token bucket derived from config (default: 30 uploads/hour).
    // per_millisecond sets the replenishment interval per token.
    let per_ip_per_hour = state.config.rate_limit.per_ip_per_hour;
    let replenish_ms = (3_600_000u64).checked_div(per_ip_per_hour as u64).unwrap_or(3_600_000);
    let governor_conf = GovernorConfigBuilder::default()
        .per_millisecond(replenish_ms)
        .burst_size(5)
        .key_extractor(SmartIpKeyExtractor)
        .finish()
        .unwrap();

    // Periodically evict expired rate-limit entries to keep memory bounded.
    let limiter = Arc::clone(governor_conf.limiter());
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            limiter.retain_recent();
        }
    });

    // Body size cap: configured max upload + headroom for multipart framing.
    let body_limit = state.config.limits.max_upload_bytes as usize + 64 * 1024;

    Router::new()
        .route("/v1/uploads", post(uploads::create_upload))
        .layer(DefaultBodyLimit::max(body_limit))
        .layer(GovernorLayer::new(governor_conf))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .with_state(state)
}

/// Meta / operational routes (no auth or rate limiting required)
pub fn meta_router(metrics: PrometheusHandle) -> Router {
    Router::new()
        .route("/health", get(health::health_check))
        .route("/metrics", get(move || async move { metrics.render() }))
}
