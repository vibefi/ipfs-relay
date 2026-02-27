/// Kubo (go-ipfs) HTTP API client
///
/// We use Kubo's `/api/v0/add` endpoint to add files as a UnixFS directory,
/// which gives us the deterministic root CID that clients use for verification.
use std::collections::HashMap;

use reqwest::multipart::{Form, Part};
use serde::Deserialize;
use tracing::{debug, instrument};

use crate::error::{AppError, IpfsError};
use crate::models::UploadedFile;

#[derive(Clone)]
pub struct KuboClient {
    base_url: String,
    http: reqwest::Client,
}

/// Response from Kubo /api/v0/add (one line per file, NDJSON)
#[derive(Debug, Deserialize)]
struct KuboAddEntry {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Hash")]
    hash: String,
}

/// Result of adding files to IPFS
pub struct AddResult {
    /// CID of the root directory
    pub root_cid: String,
    /// Map of path → CID for all added files
    pub file_cids: HashMap<String, String>,
}

impl KuboClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(90))
                .build()
                .expect("failed to build reqwest client"),
        }
    }

    /// Add a slice of files as a wrapped UnixFS directory.
    /// Returns the root CID of the directory.
    #[instrument(skip(self, files), fields(file_count = files.len()))]
    pub async fn add_directory(&self, files: &[UploadedFile]) -> Result<AddResult, AppError> {
        let mut form = Form::new();

        for f in files {
            let part = Part::bytes(f.content.to_vec())
                .file_name(f.path.clone())
                .mime_str("application/octet-stream")
                .map_err(|e| AppError::Internal(anyhow::anyhow!("multipart build error: {e}")))?;
            form = form.part("file", part);
        }

        // wrap=true → Kubo wraps files in a directory and returns the dir CID last
        let url = format!(
            "{}/api/v0/add?pin=true&wrap-with-directory=true&cid-version=1&hash=sha2-256",
            self.base_url
        );

        debug!(%url, "sending files to kubo");

        let response = self
            .http
            .post(&url)
            .multipart(form)
            .send()
            .await
            .map_err(IpfsError::Http)?;

        if !response.status().is_success() {
            let msg = response.text().await.unwrap_or_default();
            return Err(AppError::Ipfs(IpfsError::Api { message: msg }));
        }

        // Kubo returns one JSON object per line (NDJSON)
        let body = response.text().await.map_err(IpfsError::Http)?;
        let mut file_cids: HashMap<String, String> = HashMap::new();
        let mut root_cid = String::new();

        for line in body.lines() {
            if line.is_empty() {
                continue;
            }
            let entry: KuboAddEntry = serde_json::from_str(line)
                .map_err(|e| AppError::Internal(anyhow::anyhow!("kubo parse error: {e}")))?;

            if entry.name.is_empty() {
                // The wrapping directory entry has an empty name
                root_cid = entry.hash.clone();
            } else {
                file_cids.insert(entry.name.clone(), entry.hash.clone());
            }
        }

        if root_cid.is_empty() {
            return Err(AppError::Internal(anyhow::anyhow!(
                "kubo did not return a root CID"
            )));
        }

        debug!(%root_cid, "kubo add complete");
        Ok(AddResult { root_cid, file_cids })
    }

    /// Pin an already-imported CID (used after replication jobs)
    pub async fn pin_add(&self, cid: &str) -> Result<(), AppError> {
        let url = format!("{}/api/v0/pin/add?arg={cid}", self.base_url);
        let resp = self
            .http
            .post(&url)
            .send()
            .await
            .map_err(IpfsError::Http)?;

        if !resp.status().is_success() {
            let msg = resp.text().await.unwrap_or_default();
            return Err(AppError::Ipfs(IpfsError::Api { message: msg }));
        }
        Ok(())
    }
}
