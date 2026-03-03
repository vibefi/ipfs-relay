use axum::{extract::Request, middleware::Next, response::Response};
use uuid::Uuid;

tokio::task_local! {
    static REQUEST_ID: String;
}

/// Binds the request ID to task-local context so downstream error responses
/// can emit the same identifier as request logs/traces.
pub async fn request_id_context_middleware(req: Request, next: Next) -> Response {
    let request_id = req
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("req_{}", Uuid::new_v4()));

    REQUEST_ID
        .scope(request_id, async move { next.run(req).await })
        .await
}

pub fn current_request_id() -> Option<String> {
    REQUEST_ID.try_with(Clone::clone).ok()
}
