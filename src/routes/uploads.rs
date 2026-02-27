/// Upload handlers — the core of the relay service.
///
/// POST /v1/uploads  — receive a multipart VibeFi bundle
/// GET  /v1/uploads/{upload_id} — check processing / replication status
use axum::{
    body::Bytes,
    extract::{Multipart, Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use chrono::Utc;
use sha2::{Digest, Sha256};
use tracing::{info, instrument};
use ulid::Ulid;
use uuid::Uuid;

use crate::{
    error::{AppError, AppResult},
    middleware::auth::AuthContext,
    models::{
        AuthMode, PinningSummary, ReplicationEntry, ReplicaStatus, UploadRecord, UploadResponse,
        UploadStatusResponse, UploadedFile, ValidationSummary,
    },
    validation::validate_vibefi_package,
    AppState,
};

/// POST /v1/uploads
///
/// Accepts a multipart/form-data body with repeated `file` parts.
/// Each part's filename is the relative bundle path.
#[instrument(skip(state, headers, multipart), fields(upload_id))]
pub async fn create_upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    auth: AuthContext,
    mut multipart: Multipart,
) -> AppResult<(StatusCode, Json<UploadResponse>)> {
    let upload_id = format!("upl_{}", Ulid::new());
    let request_id = format!("req_{}", Uuid::new_v4());

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

    // ── Persist upload metadata ───────────────────────────────────────────────
    let source_ip_hash = hash_ip(
        headers
            .get("x-forwarded-for")
            .or_else(|| headers.get("x-real-ip"))
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown"),
    );

    let record = UploadRecord {
        upload_id: upload_id.clone(),
        root_cid: root_cid.clone(),
        source_ip_hash,
        auth_mode: auth.mode.clone(),
        bytes: total_bytes as i64,
        file_count: files.len() as i64,
        created_at: Utc::now(),
        request_id: request_id.clone(),
        replication_status: "pending".to_string(),
    };
    state.db.insert_upload(&record).await?;

    // ── Queue replication jobs ────────────────────────────────────────────────
    state
        .pinning
        .enqueue_replication(&upload_id, &root_cid)
        .await?;

    // ── Determine which replicas were queued for the response ─────────────────
    let replica_statuses = build_replica_statuses(&state.config.pinning);

    info!(
        upload_id = %upload_id,
        root_cid = %root_cid,
        bytes = total_bytes,
        file_count = files.len(),
        auth_mode = %auth.mode,
        "upload accepted"
    );

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

/// GET /v1/uploads/{upload_id}
pub async fn get_upload(
    State(state): State<AppState>,
    Path(upload_id): Path<String>,
) -> AppResult<Json<UploadStatusResponse>> {
    let record = state
        .db
        .get_upload(&upload_id)
        .await?
        .ok_or(AppError::NotFound)?;

    let jobs = state.db.get_replication_status(&upload_id).await?;

    let mut replication: Vec<ReplicationEntry> = vec![ReplicationEntry {
        target: "local".to_string(),
        status: crate::models::ReplicationStatus::Pinned,
    }];

    for (target, status_str) in jobs {
        let status = match status_str.as_str() {
            "pinned" => crate::models::ReplicationStatus::Pinned,
            "failed" => crate::models::ReplicationStatus::Failed,
            _ => crate::models::ReplicationStatus::Queued,
        };
        replication.push(ReplicationEntry { target, status });
    }

    // Overall status: completed only when all known targets are pinned
    let all_pinned = replication
        .iter()
        .all(|r| r.status == crate::models::ReplicationStatus::Pinned);
    let any_failed = replication
        .iter()
        .any(|r| r.status == crate::models::ReplicationStatus::Failed);

    let status = if all_pinned {
        "completed"
    } else if any_failed {
        "partial"
    } else {
        "pending"
    };

    Ok(Json(UploadStatusResponse {
        upload_id: record.upload_id,
        root_cid: record.root_cid,
        status: status.to_string(),
        replication,
    }))
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
    if cfg.pinata_jwt.is_some() {
        replicas.push(ReplicaStatus {
            target: "pinata".to_string(),
            status: "queued".to_string(),
        });
    }
    if cfg.foureverland_token.is_some() {
        replicas.push(ReplicaStatus {
            target: "4everland".to_string(),
            status: "queued".to_string(),
        });
    }
    replicas
}
