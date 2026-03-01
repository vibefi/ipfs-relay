use serde::Serialize;

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
