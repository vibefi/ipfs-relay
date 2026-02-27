/// Custom tracing span / response hooks for tower-http TraceLayer.
///
/// Produces structured log lines:
///   { "requestId": "...", "method": "POST", "path": "/v1/uploads",
///     "statusCode": 201, "durationMs": 342, "authMode": "anonymous" }
use axum::http::{Request, Response};
use tracing::{Level, Span};
use uuid::Uuid;

pub fn make_span<B>(request: &Request<B>) -> Span {
    let request_id = Uuid::new_v4().to_string();
    tracing::span!(
        Level::INFO,
        "http_request",
        request_id = %request_id,
        method = %request.method(),
        path = %request.uri().path(),
        status_code = tracing::field::Empty,
        duration_ms = tracing::field::Empty,
    )
}

pub fn on_response<B>(response: &Response<B>, latency: std::time::Duration, span: &Span) {
    span.record("status_code", response.status().as_u16());
    span.record("duration_ms", latency.as_millis());
    tracing::info!(parent: span, "response");
}
