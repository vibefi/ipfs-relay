/// Database layer using SQLx.
///
/// Uses SQLite by default (great for single-node; swap DSN to `postgres://`
/// for multi-replica production deployments).
use chrono::Utc;
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    Pool, Sqlite,
};
use std::str::FromStr;
use tracing::instrument;

use crate::error::AppError;
use crate::models::{AuthMode, ReplicationStatus, ReplicationTarget, UploadRecord};

#[derive(Clone)]
pub struct Database {
    pool: Pool<Sqlite>,
}

/// A pending replication job row
pub struct ReplicationJob {
    pub id: String,
    pub upload_id: String,
    pub cid: String,
    pub target: String,
    pub attempts: i64,
}

impl Database {
    pub async fn connect(url: &str) -> Result<Self, AppError> {
        // Enable foreign key enforcement — SQLite skips FK checks by default.
        let opts = SqliteConnectOptions::from_str(url)
            .map_err(AppError::Database)?
            .foreign_keys(true)
            .create_if_missing(true);

        let pool = SqlitePoolOptions::new()
            .connect_with(opts)
            .await
            .map_err(AppError::Database)?;

        Ok(Self { pool })
    }

    /// Run embedded migrations from the `migrations/` directory
    pub async fn migrate(&self) -> Result<(), AppError> {
        sqlx::migrate!("./migrations")
            .run(&self.pool)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("migration failed: {e}")))?;
        Ok(())
    }

    // ── Uploads ──────────────────────────────────────────────────────────────

    #[instrument(skip(self, record))]
    pub async fn insert_upload(&self, record: &UploadRecord) -> Result<(), AppError> {
        sqlx::query!(
            r#"
            INSERT INTO uploads
                (upload_id, root_cid, source_ip_hash, auth_mode, bytes, file_count, created_at, request_id, replication_status)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
            record.upload_id,
            record.root_cid,
            record.source_ip_hash,
            record.auth_mode as _,
            record.bytes,
            record.file_count,
            record.created_at,
            record.request_id,
            record.replication_status,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_upload(&self, upload_id: &str) -> Result<Option<UploadRecord>, AppError> {
        let row = sqlx::query!(
            r#"
            SELECT upload_id, root_cid, source_ip_hash, auth_mode as "auth_mode: String",
                   bytes, file_count, created_at as "created_at: chrono::DateTime<Utc>",
                   request_id, replication_status
            FROM uploads
            WHERE upload_id = ?
            "#,
            upload_id
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| UploadRecord {
            upload_id: r.upload_id,
            root_cid: r.root_cid,
            source_ip_hash: r.source_ip_hash,
            auth_mode: if r.auth_mode == "api_key" {
                AuthMode::ApiKey
            } else {
                AuthMode::Anonymous
            },
            bytes: r.bytes,
            file_count: r.file_count,
            created_at: r.created_at,
            request_id: r.request_id,
            replication_status: r.replication_status,
        }))
    }

    // ── Replication jobs ─────────────────────────────────────────────────────

    pub async fn create_replication_jobs(
        &self,
        upload_id: &str,
        cid: &str,
        targets: &[ReplicationTarget],
    ) -> Result<(), AppError> {
        for target in targets {
            let id = ulid::Ulid::new().to_string();
            let target_str = target.to_string();
            let now = Utc::now();
            sqlx::query!(
                r#"
                INSERT INTO replication_jobs (id, upload_id, cid, target, status, attempts, created_at)
                VALUES (?, ?, ?, ?, 'queued', 0, ?)
                "#,
                id,
                upload_id,
                cid,
                target_str,
                now,
            )
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    pub async fn get_pending_replication_jobs(
        &self,
        limit: i64,
    ) -> Result<Vec<ReplicationJob>, AppError> {
        let rows = sqlx::query!(
            r#"
            SELECT id, upload_id, cid, target, attempts
            FROM replication_jobs
            WHERE status = 'queued' AND attempts < 5
            ORDER BY created_at ASC
            LIMIT ?
            "#,
            limit
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| ReplicationJob {
                id: r.id,
                upload_id: r.upload_id,
                cid: r.cid,
                target: r.target,
                attempts: r.attempts,
            })
            .collect())
    }

    pub async fn update_replication_job(
        &self,
        job_id: &str,
        status: ReplicationStatus,
        last_error: Option<String>,
    ) -> Result<(), AppError> {
        let status_str = status.to_string();
        let now = Utc::now();
        sqlx::query!(
            r#"
            UPDATE replication_jobs
            SET status = ?, attempts = attempts + 1, last_error = ?, updated_at = ?
            WHERE id = ?
            "#,
            status_str,
            last_error,
            now,
            job_id,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_replication_status(
        &self,
        upload_id: &str,
    ) -> Result<Vec<(String, String)>, AppError> {
        let rows = sqlx::query!(
            r#"SELECT target, status FROM replication_jobs WHERE upload_id = ?"#,
            upload_id
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| (r.target, r.status)).collect())
    }

    /// Prune upload metadata older than `days` days
    pub async fn prune_old_uploads(&self, days: u32) -> Result<u64, AppError> {
        let cutoff = Utc::now() - chrono::Duration::days(days as i64);
        let result = sqlx::query!(
            r#"DELETE FROM uploads WHERE created_at < ?"#,
            cutoff
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }
}
