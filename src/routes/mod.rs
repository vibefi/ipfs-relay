pub mod health;
pub mod uploads;

use axum::{
    routing::{get, post},
    Router,
};
use axum_prometheus::metrics_exporter_prometheus::PrometheusHandle;
use axum::middleware;

use crate::AppState;
use crate::middleware::auth::auth_middleware;

/// `/v1/*` routes with auth middleware
pub fn api_router(state: AppState) -> Router {
    Router::new()
        .route("/v1/uploads", post(uploads::create_upload))
        .route("/v1/uploads/{upload_id}", get(uploads::get_upload))
        .layer(middleware::from_fn_with_state(state.clone(), auth_middleware))
        .with_state(state)
}

/// Meta / operational routes (no auth required)
pub fn meta_router(metrics: PrometheusHandle) -> Router {
    Router::new()
        .route("/health", get(health::health_check))
        .route(
            "/metrics",
            get(move || async move { metrics.render() }),
        )
}
