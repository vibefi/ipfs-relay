pub mod foureverland;
/// Async replication pipeline
///
/// After local IPFS pin completes, we enqueue replication jobs for:
///   - Pinata (protocol-owned account)
///   - 4EVERLAND (protocol-owned account)
///
/// Jobs are persisted in the DB so they survive restarts and can be retried.
/// The background worker polls the queue and attempts each job independently.
pub mod pinata;

use std::sync::Arc;
use std::time::Duration;

use tracing::{error, info, instrument, warn};

use crate::config::PinningConfig;
use crate::error::AppError;
use crate::ipfs::KuboClient;
use crate::models::{ReplicationStatus, ReplicationTarget};
use crate::storage::db::Database;

pub struct PinningService {
    db: Database,
    ipfs: Arc<KuboClient>,
    config: PinningConfig,
}

impl PinningService {
    pub fn new(db: Database, ipfs: Arc<KuboClient>, config: PinningConfig) -> Self {
        Self { db, ipfs, config }
    }

    /// Enqueue replication jobs for a newly uploaded CID.
    /// Returns immediately — jobs run in the background worker.
    pub async fn enqueue_replication(
        &self,
        upload_id: &str,
        root_cid: &str,
    ) -> Result<(), AppError> {
        let targets = self.enabled_targets();
        self.db
            .create_replication_jobs(upload_id, root_cid, &targets)
            .await?;
        Ok(())
    }

    fn enabled_targets(&self) -> Vec<ReplicationTarget> {
        let mut targets = Vec::new();
        if self.config.pinata_jwt.is_some() {
            targets.push(ReplicationTarget::Pinata);
        }
        if self.config.foureverland_token.is_some() {
            targets.push(ReplicationTarget::Foureverland);
        }
        targets
    }

    /// Spawn the background replication worker on the current tokio runtime.
    pub fn start_worker(self: Arc<Self>) {
        tokio::spawn(async move {
            info!("replication worker started");
            loop {
                if let Err(e) = self.process_pending_jobs().await {
                    error!(error = %e, "replication worker error");
                }
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
        });
    }

    #[instrument(skip(self))]
    async fn process_pending_jobs(&self) -> Result<(), AppError> {
        let jobs = self.db.get_pending_replication_jobs(50).await?;
        for job in jobs {
            let result = self.run_job(&job.target, &job.cid).await;
            match result {
                Ok(_) => {
                    info!(target = %job.target, cid = %job.cid, "replication succeeded");
                    self.db
                        .update_replication_job(&job.id, ReplicationStatus::Pinned, None)
                        .await?;
                }
                Err(e) => {
                    warn!(target = %job.target, cid = %job.cid, error = %e, attempt = job.attempts + 1, "replication failed");
                    let should_give_up = job.attempts + 1 >= 5;
                    let status = if should_give_up {
                        ReplicationStatus::Failed
                    } else {
                        ReplicationStatus::Queued
                    };
                    self.db
                        .update_replication_job(&job.id, status, Some(e.to_string()))
                        .await?;
                }
            }
        }
        Ok(())
    }

    async fn run_job(&self, target: &str, cid: &str) -> Result<(), anyhow::Error> {
        match target {
            "pinata" => {
                let jwt = self
                    .config
                    .pinata_jwt
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("pinata_jwt not configured"))?;
                pinata::pin_by_cid(jwt, cid).await
            }
            "4everland" => {
                let token = self
                    .config
                    .foureverland_token
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("foureverland_token not configured"))?;
                foureverland::pin_by_cid(token, cid).await
            }
            _ => Err(anyhow::anyhow!("unknown replication target: {target}")),
        }
    }
}
