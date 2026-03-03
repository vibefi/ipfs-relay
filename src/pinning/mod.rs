pub mod foureverland;
pub mod pinata;

use std::sync::Arc;
use std::time::Duration;

use tracing::{error, info, warn};

use crate::config::PinningConfig;

pub struct PinningService {
    config: PinningConfig,
}

impl PinningService {
    pub fn new(config: PinningConfig) -> Self {
        Self { config }
    }

    /// Fire-and-forget replication: spawn a task per enabled target.
    /// Each task retries up to 3 times with exponential backoff.
    pub fn spawn_replication(self: &Arc<Self>, root_cid: String) {
        if let Some(jwt) = self.config.pinata_jwt_value().map(String::from) {
            let cid = root_cid.clone();
            tokio::spawn(async move {
                retry_pin("pinata", &cid, 3, || pinata::pin_by_cid(&jwt, &cid)).await;
            });
        }

        if let Some(token) = self.config.foureverland_token_value().map(String::from) {
            let cid = root_cid;
            tokio::spawn(async move {
                retry_pin("4everland", &cid, 3, || {
                    foureverland::pin_by_cid(&token, &cid)
                })
                .await;
            });
        }
    }
}

async fn retry_pin<F, Fut>(target: &str, cid: &str, max_attempts: u32, f: F)
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<(), anyhow::Error>>,
{
    for attempt in 1..=max_attempts {
        match f().await {
            Ok(()) => {
                info!(target = target, %cid, "replication succeeded");
                return;
            }
            Err(e) => {
                if attempt == max_attempts {
                    error!(target = target, %cid, error = %e, "replication failed after {max_attempts} attempts");
                } else {
                    warn!(target = target, %cid, error = %e, attempt, "replication attempt failed, retrying");
                    tokio::time::sleep(Duration::from_secs(2u64.pow(attempt))).await;
                }
            }
        }
    }
}
