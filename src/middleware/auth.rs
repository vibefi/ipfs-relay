/// Auth middleware / extractor
///
/// Current policy (spec § 6):
///   - Anonymous uploads accepted with strict quotas
///   - Optional API key via `Authorization: Bearer <key>` for higher quotas
///   - No wallet-signature requirement yet
use axum::{
    extract::{FromRequestParts, Request},
    http::{header, request::Parts, StatusCode},
    middleware::Next,
    response::Response,
};

use crate::error::AppError;
use crate::models::AuthMode;

/// Resolved auth context, injected as an extension by `auth_middleware`.
#[derive(Clone, Debug)]
pub struct AuthContext {
    pub mode: AuthMode,
    /// The API key that was used (not logged; only used for quota checks)
    pub api_key: Option<String>,
}

/// Axum layer that resolves auth and injects an `AuthContext` extension.
/// Always succeeds — even anonymous requests are allowed.
pub async fn auth_middleware(
    axum::extract::State(state): axum::extract::State<crate::AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, AppError> {
    let bearer = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(String::from);

    let ctx = match bearer {
        Some(key) => {
            let valid_keys = state.config.api_keys();
            if valid_keys.is_empty() || valid_keys.contains(&key) {
                AuthContext {
                    mode: AuthMode::ApiKey,
                    api_key: Some(key),
                }
            } else {
                return Err(AppError::Unauthorized("invalid API key".into()));
            }
        }
        None => AuthContext {
            mode: AuthMode::Anonymous,
            api_key: None,
        },
    };

    req.extensions_mut().insert(ctx);
    Ok(next.run(req).await)
}

/// Extractor that pulls `AuthContext` from request extensions.
#[axum::async_trait]
impl<S> FromRequestParts<S> for AuthContext
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthContext>()
            .cloned()
            .ok_or((StatusCode::INTERNAL_SERVER_ERROR, "auth context missing"))
    }
}
