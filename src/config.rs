use anyhow::Result;
use config::{Config, Environment, File};
use serde::Deserialize;

/// Top-level application configuration.
/// Sources (lowest → highest priority):
///   1. `config/default.toml`
///   2. `config/{APP_ENV}.toml`  (APP_ENV defaults to "development")
///   3. Environment variables prefixed with `VIBEFI_RELAY_`
#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub ipfs: IpfsConfig,
    pub pinning: PinningConfig,
    pub limits: LimitsConfig,
    pub rate_limit: RateLimitConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    /// Overall request timeout in seconds (spec: 120s)
    #[serde(default = "default_timeout")]
    pub request_timeout_secs: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IpfsConfig {
    #[serde(default = "default_kubo_url")]
    pub kubo_api_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PinningConfig {
    pub pinata_jwt: Option<String>,
    pub foureverland_token: Option<String>,
}

impl PinningConfig {
    pub fn pinata_jwt_value(&self) -> Option<&str> {
        self.pinata_jwt
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }

    pub fn foureverland_token_value(&self) -> Option<&str> {
        self.foureverland_token
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct LimitsConfig {
    /// Max total upload bytes (spec: 10 MiB)
    #[serde(default = "default_max_bytes")]
    pub max_upload_bytes: u64,
    /// Max number of files per upload (spec: 1500)
    #[serde(default = "default_max_files")]
    pub max_file_count: usize,
    /// Max single file size (spec: 5 MiB)
    #[serde(default = "default_max_file_bytes")]
    pub max_single_file_bytes: u64,
    /// Reject files not listed in manifest.files (optional strict mode)
    #[serde(default)]
    pub strict_manifest: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RateLimitConfig {
    /// Uploads per minute per IP
    #[serde(default = "default_ip_per_minute")]
    pub per_ip_per_minute: u32,
    /// Uploads per hour per IP (anonymous)
    #[serde(default = "default_ip_per_hour")]
    pub per_ip_per_hour: u32,
}

impl AppConfig {
    pub fn load() -> Result<Self> {
        let env = std::env::var("APP_ENV").unwrap_or_else(|_| "development".into());

        let cfg = Config::builder()
            .add_source(File::with_name("config/default").required(false))
            .add_source(File::with_name(&format!("config/{env}")).required(false))
            .add_source(
                // Use __ as the nesting separator so single-underscore field names
                // (e.g. kubo_api_url, pinata_jwt) are not split incorrectly.
                // Env vars must use __ between nesting levels:
                //   VIBEFI_RELAY__IPFS__KUBO_API_URL=http://kubo:5001
                //   VIBEFI_RELAY__PINNING__PINATA_JWT=...
                Environment::with_prefix("VIBEFI_RELAY")
                    .prefix_separator("__")
                    .separator("__")
                    .try_parsing(true),
            )
            .build()?;

        Ok(cfg.try_deserialize()?)
    }
}

fn default_host() -> String {
    "0.0.0.0".into()
}
fn default_port() -> u16 {
    8080
}
fn default_timeout() -> u64 {
    120
}
fn default_kubo_url() -> String {
    "http://127.0.0.1:5001".into()
}
fn default_max_bytes() -> u64 {
    10 * 1024 * 1024
} // 10 MiB
fn default_max_files() -> usize {
    1500
}
fn default_max_file_bytes() -> u64 {
    5 * 1024 * 1024
} // 5 MiB
fn default_ip_per_minute() -> u32 {
    1
}
fn default_ip_per_hour() -> u32 {
    15
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    /// Helper: set env vars, run a closure, then clean up.
    fn with_env_vars<F: FnOnce()>(vars: &[(&str, &str)], f: F) {
        let _lock = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: Tests are serialised by ENV_MUTEX so no other thread is
        // reading env vars concurrently.
        for (k, v) in vars {
            unsafe { std::env::set_var(k, v) };
        }
        f();
        for (k, _) in vars {
            unsafe { std::env::remove_var(k) };
        }
    }

    #[test]
    fn env_overrides_all_sections() {
        with_env_vars(
            &[
                ("VIBEFI_RELAY__SERVER__PORT", "9999"),
                (
                    "VIBEFI_RELAY__IPFS__KUBO_API_URL",
                    "http://custom-kubo:5001",
                ),
                ("VIBEFI_RELAY__PINNING__PINATA_JWT", "test-jwt-token"),
                ("VIBEFI_RELAY__LIMITS__MAX_UPLOAD_BYTES", "2048"),
                ("VIBEFI_RELAY__RATE_LIMIT__PER_IP_PER_HOUR", "100"),
            ],
            || {
                let cfg = AppConfig::load().expect("config should load");
                assert_eq!(cfg.server.port, 9999);
                assert_eq!(cfg.ipfs.kubo_api_url, "http://custom-kubo:5001");
                assert_eq!(cfg.pinning.pinata_jwt.as_deref(), Some("test-jwt-token"));
                assert_eq!(cfg.limits.max_upload_bytes, 2048);
                assert_eq!(cfg.rate_limit.per_ip_per_hour, 100);
            },
        );
    }

    #[test]
    fn env_overrides_take_precedence_over_toml() {
        // default.toml sets port = 8080
        with_env_vars(&[("VIBEFI_RELAY__SERVER__PORT", "3000")], || {
            let cfg = AppConfig::load().expect("config should load");
            assert_eq!(
                cfg.server.port, 3000,
                "env var should override toml default"
            );
        });
    }

    #[test]
    fn single_underscore_field_names_preserved() {
        with_env_vars(
            &[
                (
                    "VIBEFI_RELAY__IPFS__KUBO_API_URL",
                    "http://underscore-test:5001",
                ),
                (
                    "VIBEFI_RELAY__PINNING__FOUREVERLAND_TOKEN",
                    "test-4ever-token",
                ),
            ],
            || {
                let cfg = AppConfig::load().expect("config should load");
                assert_eq!(cfg.ipfs.kubo_api_url, "http://underscore-test:5001");
                assert_eq!(
                    cfg.pinning.foureverland_token.as_deref(),
                    Some("test-4ever-token"),
                );
            },
        );
    }
}
