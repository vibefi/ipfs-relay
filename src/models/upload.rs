use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Auth mode for an upload request
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "camelCase")]
#[sqlx(rename_all = "snake_case", type_name = "text")]
pub enum AuthMode {
    Anonymous,
    ApiKey,
}

impl std::fmt::Display for AuthMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Anonymous => write!(f, "anonymous"),
            Self::ApiKey => write!(f, "apiKey"),
        }
    }
}

/// Replication target name
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReplicationTarget {
    Local,
    Pinata,
    Foureverland,
}

impl std::fmt::Display for ReplicationTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local => write!(f, "local"),
            Self::Pinata => write!(f, "pinata"),
            Self::Foureverland => write!(f, "4everland"),
        }
    }
}

/// Status of a single replication target
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplicationStatus {
    Queued,
    Pinned,
    Failed,
}

impl std::fmt::Display for ReplicationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Queued => write!(f, "queued"),
            Self::Pinned => write!(f, "pinned"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

/// One entry in the replication status list
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicationEntry {
    pub target: String,
    pub status: ReplicationStatus,
}

/// Persisted upload record (maps to the `uploads` table)
#[derive(Debug, Clone)]
pub struct UploadRecord {
    pub upload_id: String,
    pub root_cid: String,
    pub source_ip_hash: String,
    pub auth_mode: AuthMode,
    pub bytes: i64,
    pub file_count: i64,
    pub created_at: DateTime<Utc>,
    pub request_id: String,
    /// JSON-encoded Vec<ReplicationEntry>
    pub replication_status: String,
}

/// Response body for POST /v1/uploads (201)
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadResponse {
    pub upload_id: String,
    pub root_cid: String,
    pub bytes: u64,
    pub file_count: usize,
    pub validation: ValidationSummary,
    pub pinning: PinningSummary,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationSummary {
    pub is_vibe_fi_package: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinningSummary {
    pub local: String,
    pub replicas: Vec<ReplicaStatus>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplicaStatus {
    pub target: String,
    pub status: String,
}

/// Response body for GET /v1/uploads/{uploadId}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadStatusResponse {
    pub upload_id: String,
    pub root_cid: String,
    pub status: String,
    pub replication: Vec<ReplicationEntry>,
}

/// An uploaded file extracted from multipart
#[derive(Debug, Clone)]
pub struct UploadedFile {
    /// Relative path within the bundle (e.g. "src/App.tsx")
    pub path: String,
    pub content: bytes::Bytes,
}
