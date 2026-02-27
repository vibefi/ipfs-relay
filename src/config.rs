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
    pub database: DatabaseConfig,
    pub ipfs: IpfsConfig,
    pub pinning: PinningConfig,
    pub limits: LimitsConfig,
    pub rate_limit: RateLimitConfig,
    pub auth: AuthConfig,
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
pub struct DatabaseConfig {
    #[serde(default = "default_db_url")]
    pub url: String,
    /// Days to retain upload metadata (spec: 90)
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,
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
    /// Uploads per hour per IP (anonymous)
    #[serde(default = "default_ip_per_hour")]
    pub per_ip_per_hour: u32,
    /// Uploads per day per API key
    #[serde(default = "default_key_per_day")]
    pub per_key_per_day: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthConfig {
    /// Comma-separated list of valid API keys; empty = no keys configured
    pub api_keys: Option<String>,
}

impl AppConfig {
    pub fn load() -> Result<Self> {
        let env = std::env::var("APP_ENV").unwrap_or_else(|_| "development".into());

        let cfg = Config::builder()
            .add_source(File::with_name("config/default").required(false))
            .add_source(File::with_name(&format!("config/{env}")).required(false))
            .add_source(
                Environment::with_prefix("VIBEFI_RELAY")
                    .separator("_")
                    .try_parsing(true),
            )
            .build()?;

        Ok(cfg.try_deserialize()?)
    }

    /// Returns the set of valid API keys parsed from the config.
    pub fn api_keys(&self) -> Vec<String> {
        self.auth
            .api_keys
            .as_deref()
            .unwrap_or("")
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect()
    }
}

fn default_host() -> String { "0.0.0.0".into() }
fn default_port() -> u16 { 8080 }
fn default_timeout() -> u64 { 120 }
fn default_db_url() -> String { "sqlite://ipfs-relay.db".into() }
fn default_retention_days() -> u32 { 90 }
fn default_kubo_url() -> String { "http://127.0.0.1:5001".into() }
fn default_max_bytes() -> u64 { 10 * 1024 * 1024 }   // 10 MiB
fn default_max_files() -> usize { 1500 }
fn default_max_file_bytes() -> u64 { 5 * 1024 * 1024 } // 5 MiB
fn default_ip_per_hour() -> u32 { 30 }
fn default_key_per_day() -> u32 { 300 }
