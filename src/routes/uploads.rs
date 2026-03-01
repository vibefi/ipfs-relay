/// Upload handler — the core of the relay service.
///
/// POST /v1/uploads  — receive a multipart VibeFi bundle
use axum::{
    Json,
    body::Bytes,
    extract::{Multipart, State},
    http::{HeaderMap, StatusCode},
};
use sha2::{Digest, Sha256};
use tracing::{info, instrument};
use ulid::Ulid;
use uuid::Uuid;

use crate::{
    AppState,
    error::{AppError, AppResult},
    models::{PinningSummary, ReplicaStatus, UploadResponse, UploadedFile, ValidationSummary},
    validation::validate_vibefi_package,
};

/// POST /v1/uploads
///
/// Accepts a multipart/form-data body with repeated `file` parts.
/// Each part's filename is the relative bundle path.
#[instrument(skip(state, headers, multipart), fields(upload_id))]
pub async fn create_upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> AppResult<(StatusCode, Json<UploadResponse>)> {
    let upload_id = format!("upl_{}", Ulid::new());
    let request_id = headers
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(String::from)
        .unwrap_or_else(|| format!("req_{}", Uuid::new_v4()));

    tracing::Span::current().record("upload_id", &upload_id);

    // ── Extract multipart files ───────────────────────────────────────────────
    let mut files: Vec<UploadedFile> = Vec::new();
    let mut total_bytes: u64 = 0;

    // Enforce overall size limit before we collect everything in memory
    let max_bytes = state.config.limits.max_upload_bytes;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::InvalidPackage(format!("multipart error: {e}")))?
    {
        // Only accept fields named "file"
        if field.name() != Some("file") {
            continue;
        }

        let path = field
            .file_name()
            .ok_or_else(|| AppError::InvalidPackage("file part missing filename".into()))?
            .to_string();

        let content: Bytes = field
            .bytes()
            .await
            .map_err(|e| AppError::InvalidPackage(format!("read error for {path}: {e}")))?;

        total_bytes += content.len() as u64;
        if total_bytes > max_bytes {
            return Err(AppError::PayloadTooLarge(format!(
                "upload exceeds {max_bytes} bytes"
            )));
        }

        files.push(UploadedFile { path, content });
    }

    // ── Package validation (spec § 5) ────────────────────────────────────────
    validate_vibefi_package(&files, &state.config.limits)?;

    // ── IPFS add ─────────────────────────────────────────────────────────────
    let add_result = state.ipfs.add_directory(&files).await?;
    let root_cid = add_result.root_cid.clone();

    // ── Log upload metadata (replaces DB persistence) ────────────────────────
    let source_ip_hash = hash_ip(
        headers
            .get("x-forwarded-for")
            .or_else(|| headers.get("x-real-ip"))
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown"),
    );

    info!(
        upload_id = %upload_id,
        request_id = %request_id,
        root_cid = %root_cid,
        bytes = total_bytes,
        file_count = files.len(),
        source_ip_hash = %source_ip_hash,
        "upload accepted"
    );

    // ── Fire-and-forget replication to remote pinning services ───────────────
    state.pinning.spawn_replication(root_cid.clone());

    // ── Determine which replicas were queued for the response ─────────────────
    let replica_statuses = build_replica_statuses(&state.config.pinning);

    let response = UploadResponse {
        upload_id,
        root_cid,
        bytes: total_bytes,
        file_count: files.len(),
        validation: ValidationSummary {
            is_vibe_fi_package: true,
        },
        pinning: PinningSummary {
            local: "pinned".to_string(),
            replicas: replica_statuses,
        },
    };

    Ok((StatusCode::CREATED, Json(response)))
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn hash_ip(ip: &str) -> String {
    // Truncate/hash IP for privacy per spec § 9
    let mut hasher = Sha256::new();
    hasher.update(ip.as_bytes());
    hasher.update(b"vibefi-relay-salt");
    let result = hasher.finalize();
    hex::encode(&result[..8]) // first 8 bytes is plenty for abuse correlation
}

fn build_replica_statuses(cfg: &crate::config::PinningConfig) -> Vec<ReplicaStatus> {
    let mut replicas = Vec::new();
    if cfg.pinata_jwt_value().is_some() {
        replicas.push(ReplicaStatus {
            target: "pinata".to_string(),
            status: "queued".to_string(),
        });
    }
    if cfg.foureverland_token_value().is_some() {
        replicas.push(ReplicaStatus {
            target: "4everland".to_string(),
            status: "queued".to_string(),
        });
    }
    replicas
}
