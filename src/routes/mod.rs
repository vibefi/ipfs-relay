pub mod health;
pub mod uploads;

use std::sync::Arc;
use std::time::Duration;

use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{get, post},
};
use axum_prometheus::metrics_exporter_prometheus::PrometheusHandle;
use tower_governor::{
    GovernorLayer, governor::GovernorConfigBuilder, key_extractor::SmartIpKeyExtractor,
};

use crate::AppState;

/// `/v1/*` routes with rate limiting and body size cap.
pub fn api_router(state: AppState) -> Router {
    // IP-based token buckets:
    // - 1 request per minute (burst 1)
    // - configured requests per hour (burst max(3, per_hour / 3))
    let per_ip_per_minute = state.config.rate_limit.per_ip_per_minute;
    let per_ip_per_hour = state.config.rate_limit.per_ip_per_hour;
    let hourly_burst = (per_ip_per_hour / 3).max(3);
    let replenish_minute_ms = (60_000u64)
        .checked_div(per_ip_per_minute as u64)
        .unwrap_or(60_000)
        .max(1);
    let replenish_hour_ms = (3_600_000u64)
        .checked_div(per_ip_per_hour as u64)
        .unwrap_or(3_600_000)
        .max(1);

    let minute_conf = GovernorConfigBuilder::default()
        .per_millisecond(replenish_minute_ms)
        .burst_size(1)
        .key_extractor(SmartIpKeyExtractor)
        .finish()
        .unwrap();

    let hour_conf = GovernorConfigBuilder::default()
        .per_millisecond(replenish_hour_ms)
        .burst_size(hourly_burst)
        .key_extractor(SmartIpKeyExtractor)
        .finish()
        .unwrap();

    // Periodically evict expired rate-limit entries to keep memory bounded.
    let minute_limiter = Arc::clone(minute_conf.limiter());
    let hour_limiter = Arc::clone(hour_conf.limiter());
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            minute_limiter.retain_recent();
            hour_limiter.retain_recent();
        }
    });

    // Body size cap: configured max upload + headroom for multipart framing.
    let body_limit = state.config.limits.max_upload_bytes as usize + 64 * 1024;

    Router::new()
        .route("/v1/uploads", post(uploads::create_upload))
        .layer(DefaultBodyLimit::max(body_limit))
        .layer(GovernorLayer::new(hour_conf))
        .layer(GovernorLayer::new(minute_conf))
        .with_state(state)
}

/// Meta / operational routes (no auth or rate limiting required)
pub fn meta_router(metrics: PrometheusHandle) -> Router {
    Router::new()
        .route("/health", get(health::health_check))
        .route("/metrics", get(move || async move { metrics.render() }))
}
