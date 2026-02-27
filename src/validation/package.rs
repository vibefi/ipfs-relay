/// VibeFi package validation (spec § 5)
///
/// Rules enforced:
///  1. Total payload ≤ max_upload_bytes
///  2. At least one file present
///  3. manifest.json present and valid JSON
///  4. manifest.json has required fields: name, version, createdAt, layout, entry, files
///  5. Every manifest.files[] entry exists in upload and declared bytes match actual bytes
///  6. vibefi.json present
///  7. entry path from manifest exists
///  8. No absolute paths, ".." components, empty segments, or duplicate logical paths
///  9. (Optional strict mode) no files present that are not listed in manifest
use std::collections::HashMap;

use serde::Deserialize;
use tracing::debug;

use crate::config::LimitsConfig;
use crate::error::AppError;
use crate::models::UploadedFile;

#[derive(Debug, Deserialize)]
struct Manifest {
    name: serde_json::Value,
    version: serde_json::Value,
    #[serde(rename = "createdAt")]
    created_at: serde_json::Value,
    layout: serde_json::Value,
    entry: String,
    files: Vec<ManifestFileEntry>,
}

#[derive(Debug, Deserialize)]
struct ManifestFileEntry {
    path: String,
    bytes: u64,
}

/// Validate the uploaded file set against VibeFi package rules.
/// Returns `Ok(())` on success or `Err(AppError::InvalidPackage(...))`.
pub fn validate_vibefi_package(
    files: &[UploadedFile],
    limits: &LimitsConfig,
) -> Result<(), AppError> {
    // ── Rule 8: path sanity on every file ────────────────────────────────────
    let mut seen_paths: HashMap<String, ()> = HashMap::new();
    for f in files {
        validate_path(&f.path)?;
        if seen_paths.insert(f.path.clone(), ()).is_some() {
            return Err(AppError::InvalidPackage(format!(
                "duplicate path: {}",
                f.path
            )));
        }
    }

    // ── Rule 1: total size ───────────────────────────────────────────────────
    let total: u64 = files.iter().map(|f| f.content.len() as u64).sum();
    if total > limits.max_upload_bytes {
        return Err(AppError::PayloadTooLarge(format!(
            "upload is {} bytes, max is {} bytes",
            total, limits.max_upload_bytes
        )));
    }

    // ── Rule 10: per-file size ───────────────────────────────────────────────
    for f in files {
        if f.content.len() as u64 > limits.max_single_file_bytes {
            return Err(AppError::PayloadTooLarge(format!(
                "file {} is {} bytes, max per file is {} bytes",
                f.path,
                f.content.len(),
                limits.max_single_file_bytes
            )));
        }
    }

    // ── Rule 11: file count ──────────────────────────────────────────────────
    if files.len() > limits.max_file_count {
        return Err(AppError::InvalidPackage(format!(
            "too many files: {} (max {})",
            files.len(),
            limits.max_file_count
        )));
    }

    // ── Rule 2: at least one file ────────────────────────────────────────────
    if files.is_empty() {
        return Err(AppError::InvalidPackage("no files in upload".into()));
    }

    // Build a lookup map of path → file
    let file_map: HashMap<&str, &UploadedFile> =
        files.iter().map(|f| (f.path.as_str(), f)).collect();

    // ── Rule 6: vibefi.json must exist ──────────────────────────────────────
    if !file_map.contains_key("vibefi.json") {
        return Err(AppError::InvalidPackage(
            "vibefi.json missing from bundle root".into(),
        ));
    }

    // ── Rule 3: manifest.json must exist and parse ───────────────────────────
    let manifest_file = file_map
        .get("manifest.json")
        .ok_or_else(|| AppError::InvalidPackage("manifest.json missing from bundle root".into()))?;

    let manifest: Manifest = serde_json::from_slice(&manifest_file.content).map_err(|e| {
        AppError::InvalidPackage(format!("manifest.json failed to parse: {e}"))
    })?;

    // ── Rule 4: required fields (presence already enforced by Deserialize) ───
    // Verify non-null primitives
    for (field, val) in [
        ("name", &manifest.name),
        ("version", &manifest.version),
        ("createdAt", &manifest.created_at),
        ("layout", &manifest.layout),
    ] {
        if val.is_null() {
            return Err(AppError::InvalidPackage(format!(
                "manifest.json missing required field: {field}"
            )));
        }
    }

    // ── Rule 7: entry path must exist ───────────────────────────────────────
    if !file_map.contains_key(manifest.entry.as_str()) {
        return Err(AppError::InvalidPackage(format!(
            "entry '{}' declared in manifest.json does not exist in upload",
            manifest.entry
        )));
    }

    // ── Rule 5: manifest.files[] cross-check ────────────────────────────────
    for mf in &manifest.files {
        validate_path(&mf.path)?;
        let uploaded = file_map.get(mf.path.as_str()).ok_or_else(|| {
            AppError::InvalidPackage(format!(
                "manifest.json lists '{}' but it was not uploaded",
                mf.path
            ))
        })?;
        let actual = uploaded.content.len() as u64;
        if actual != mf.bytes {
            return Err(AppError::InvalidPackage(format!(
                "file '{}': manifest declares {} bytes but upload contains {} bytes",
                mf.path, mf.bytes, actual
            )));
        }
    }

    // ── Rule 9 (strict mode): no unlisted files ──────────────────────────────
    if limits.strict_manifest {
        let manifest_paths: std::collections::HashSet<&str> =
            manifest.files.iter().map(|f| f.path.as_str()).collect();
        // vibefi.json and manifest.json themselves are exempt
        let exempt = ["manifest.json", "vibefi.json"];
        for f in files {
            if !exempt.contains(&f.path.as_str()) && !manifest_paths.contains(f.path.as_str()) {
                return Err(AppError::InvalidPackage(format!(
                    "strict mode: '{}' is not listed in manifest.files",
                    f.path
                )));
            }
        }
    }

    debug!(
        file_count = files.len(),
        total_bytes = total,
        "package validation passed"
    );

    Ok(())
}

/// Check that a path component is safe (no absolute paths, no "..", no empty segments)
fn validate_path(path: &str) -> Result<(), AppError> {
    if path.starts_with('/') {
        return Err(AppError::InvalidPackage(format!(
            "absolute path not allowed: {path}"
        )));
    }
    for segment in path.split('/') {
        if segment.is_empty() {
            return Err(AppError::InvalidPackage(format!(
                "empty path segment in: {path}"
            )));
        }
        if segment == ".." {
            return Err(AppError::InvalidPackage(format!(
                "directory traversal not allowed: {path}"
            )));
        }
    }
    // Must be valid UTF-8 (already guaranteed by Rust &str)
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    fn make_limits() -> LimitsConfig {
        LimitsConfig {
            max_upload_bytes: 10 * 1024 * 1024,
            max_file_count: 1500,
            max_single_file_bytes: 5 * 1024 * 1024,
            strict_manifest: false,
        }
    }

    fn file(path: &str, content: &[u8]) -> UploadedFile {
        UploadedFile {
            path: path.to_string(),
            content: Bytes::copy_from_slice(content),
        }
    }

    fn valid_manifest(entry: &str) -> Vec<u8> {
        let entry_bytes = b"hello";
        serde_json::to_vec(&serde_json::json!({
            "name": "test-app",
            "version": "1.0.0",
            "createdAt": "2026-01-01T00:00:00Z",
            "layout": "spa",
            "entry": entry,
            "files": [
                { "path": entry, "bytes": entry_bytes.len() }
            ]
        }))
        .unwrap()
    }

    #[test]
    fn valid_package_passes() {
        let manifest = valid_manifest("index.html");
        let files = vec![
            file("manifest.json", &manifest),
            file("vibefi.json", b"{}"),
            file("index.html", b"hello"),
        ];
        assert!(validate_vibefi_package(&files, &make_limits()).is_ok());
    }

    #[test]
    fn missing_vibefi_json_fails() {
        let manifest = valid_manifest("index.html");
        let files = vec![
            file("manifest.json", &manifest),
            file("index.html", b"hello"),
        ];
        let err = validate_vibefi_package(&files, &make_limits()).unwrap_err();
        assert!(matches!(err, AppError::InvalidPackage(_)));
    }

    #[test]
    fn absolute_path_rejected() {
        let err = validate_path("/etc/passwd").unwrap_err();
        assert!(matches!(err, AppError::InvalidPackage(_)));
    }

    #[test]
    fn dotdot_rejected() {
        let err = validate_path("../../secret").unwrap_err();
        assert!(matches!(err, AppError::InvalidPackage(_)));
    }
}
