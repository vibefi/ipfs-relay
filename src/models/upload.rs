use serde::{Deserialize, Serialize};

/// Auth mode for an upload request
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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

/// An uploaded file extracted from multipart
#[derive(Debug, Clone)]
pub struct UploadedFile {
    /// Relative path within the bundle (e.g. "src/App.tsx")
    pub path: String,
    pub content: bytes::Bytes,
}
