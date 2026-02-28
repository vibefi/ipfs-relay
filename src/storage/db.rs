/// Database layer using SQLx.
///
/// Uses SQLite by default (great for single-node; swap DSN to `postgres://`
/// for multi-replica production deployments).
use chrono::Utc;
use sqlx::{
    Pool, Row, Sqlite,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
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
        let auth_mode = match record.auth_mode {
            AuthMode::Anonymous => "anonymous",
            AuthMode::ApiKey => "api_key",
        };

        sqlx::query(
            r#"
            INSERT INTO uploads
                (upload_id, root_cid, source_ip_hash, auth_mode, bytes, file_count, created_at, request_id, replication_status)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&record.upload_id)
        .bind(&record.root_cid)
        .bind(&record.source_ip_hash)
        .bind(auth_mode)
        .bind(record.bytes)
        .bind(record.file_count)
        .bind(record.created_at)
        .bind(&record.request_id)
        .bind(&record.replication_status)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_upload(&self, upload_id: &str) -> Result<Option<UploadRecord>, AppError> {
        let row = sqlx::query(
            r#"
            SELECT upload_id, root_cid, source_ip_hash, auth_mode,
                   bytes, file_count, created_at,
                   request_id, replication_status
            FROM uploads
            WHERE upload_id = ?
            "#,
        )
        .bind(upload_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row
            .map(|r| -> Result<UploadRecord, sqlx::Error> {
                let auth_mode: String = r.try_get("auth_mode")?;
                Ok(UploadRecord {
                    upload_id: r.try_get("upload_id")?,
                    root_cid: r.try_get("root_cid")?,
                    source_ip_hash: r.try_get("source_ip_hash")?,
                    auth_mode: if auth_mode == "api_key" {
                        AuthMode::ApiKey
                    } else {
                        AuthMode::Anonymous
                    },
                    bytes: r.try_get("bytes")?,
                    file_count: r.try_get("file_count")?,
                    created_at: r.try_get("created_at")?,
                    request_id: r.try_get("request_id")?,
                    replication_status: r.try_get("replication_status")?,
                })
            })
            .transpose()?)
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
            sqlx::query(
                r#"
                INSERT INTO replication_jobs (id, upload_id, cid, target, status, attempts, created_at)
                VALUES (?, ?, ?, ?, 'queued', 0, ?)
                "#,
            )
            .bind(id)
            .bind(upload_id)
            .bind(cid)
            .bind(target_str)
            .bind(now)
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    pub async fn get_pending_replication_jobs(
        &self,
        limit: i64,
    ) -> Result<Vec<ReplicationJob>, AppError> {
        let rows = sqlx::query(
            r#"
            SELECT id, upload_id, cid, target, attempts
            FROM replication_jobs
            WHERE status = 'queued' AND attempts < 5
            ORDER BY created_at ASC
            LIMIT ?
            "#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| {
                Ok(ReplicationJob {
                    id: r.try_get("id")?,
                    upload_id: r.try_get("upload_id")?,
                    cid: r.try_get("cid")?,
                    target: r.try_get("target")?,
                    attempts: r.try_get("attempts")?,
                })
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()?)
    }

    pub async fn update_replication_job(
        &self,
        job_id: &str,
        status: ReplicationStatus,
        last_error: Option<String>,
    ) -> Result<(), AppError> {
        let status_str = status.to_string();
        let now = Utc::now();
        sqlx::query(
            r#"
            UPDATE replication_jobs
            SET status = ?, attempts = attempts + 1, last_error = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(status_str)
        .bind(last_error)
        .bind(now)
        .bind(job_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_replication_status(
        &self,
        upload_id: &str,
    ) -> Result<Vec<(String, String)>, AppError> {
        let rows =
            sqlx::query(r#"SELECT target, status FROM replication_jobs WHERE upload_id = ?"#)
                .bind(upload_id)
                .fetch_all(&self.pool)
                .await?;

        Ok(rows
            .into_iter()
            .map(|r| Ok((r.try_get("target")?, r.try_get("status")?)))
            .collect::<Result<Vec<_>, sqlx::Error>>()?)
    }

    /// Prune upload metadata older than `days` days
    pub async fn prune_old_uploads(&self, days: u32) -> Result<u64, AppError> {
        let cutoff = Utc::now() - chrono::Duration::days(days as i64);
        let result = sqlx::query(r#"DELETE FROM uploads WHERE created_at < ?"#)
            .bind(cutoff)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }
}
