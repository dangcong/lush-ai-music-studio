//! Job queue state transitions shared by the website API, the worker protocol and the reaper.
//!
//! States: queued -> preparing -> running -> completed | failed | cancelled.
//! See docs/WORKER_PROTOCOL.md for the full state machine.

use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use sqlx::{FromRow, SqlitePool, types::Json as DbJson};

use crate::{AppState, error::AppResult, util::now};

pub const JOB_SELECT: &str = "SELECT j.id, j.type, j.status, j.priority, j.progress, j.stage, j.message, j.error, \
    j.attempt, j.max_attempts, j.cancel_requested, j.owner_id, u.username AS owner_name, j.node_id, \
    n.name AS node_name, t.id AS track_id, j.title, j.request, j.result, j.created_at, j.started_at, j.finished_at \
    FROM jobs j JOIN users u ON u.id = j.owner_id LEFT JOIN nodes n ON n.id = j.node_id \
    LEFT JOIN tracks t ON t.job_id = j.id AND t.deleted_at IS NULL";

#[derive(Serialize, FromRow)]
pub struct Job {
    pub id: String,
    #[serde(rename = "type")]
    #[sqlx(rename = "type")]
    pub job_type: String,
    pub status: String,
    pub priority: i64,
    pub progress: f64,
    pub stage: Option<String>,
    pub message: Option<String>,
    pub error: Option<String>,
    pub attempt: i64,
    pub max_attempts: i64,
    pub cancel_requested: bool,
    pub owner_id: String,
    pub owner_name: String,
    pub node_id: Option<String>,
    pub node_name: Option<String>,
    pub track_id: Option<String>,
    pub title: String,
    pub request: DbJson<Value>,
    pub result: Option<DbJson<Value>>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

pub async fn load_job(db: &SqlitePool, id: &str) -> AppResult<Option<Job>> {
    Ok(sqlx::query_as(&format!("{JOB_SELECT} WHERE j.id = ?"))
        .bind(id)
        .fetch_optional(db)
        .await?)
}

/// Take in-flight jobs away from a node that can no longer be trusted to finish them.
///
/// With `node_id` this recovers every job that node holds (worker restarted); without it, every job whose
/// lease has expired (worker crashed or lost the network). A job goes back to `queued` while it has
/// attempts left, becomes `cancelled` if the user already asked for that, and `failed` otherwise.
pub async fn recover_jobs(db: &SqlitePool, node_id: Option<&str>, reason: &str) -> AppResult<u64> {
    let now = now();
    let done = sqlx::query(
        "UPDATE jobs SET \
           status = CASE WHEN cancel_requested = 1 THEN 'cancelled' WHEN attempt < max_attempts THEN 'queued' ELSE 'failed' END, \
           error = CASE WHEN cancel_requested = 0 AND attempt >= max_attempts THEN ?1 ELSE NULL END, \
           finished_at = CASE WHEN cancel_requested = 0 AND attempt < max_attempts THEN NULL ELSE ?2 END, \
           node_id = CASE WHEN cancel_requested = 0 AND attempt < max_attempts THEN NULL ELSE node_id END, \
           log = log || '[control-plane] ' || ?1 || char(10), \
           lease_token = NULL, lease_expires_at = NULL, progress = 0, stage = NULL, message = NULL, updated_at = ?2 \
         WHERE status IN ('preparing', 'running') \
           AND ((?3 IS NOT NULL AND node_id = ?3) OR (?3 IS NULL AND lease_expires_at < ?2))",
    )
    .bind(reason)
    .bind(&now)
    .bind(node_id)
    .execute(db)
    .await?;
    Ok(done.rows_affected())
}

/// Background task: requeue or fail jobs whose lease expired.
pub fn spawn_reaper(state: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(10));
        loop {
            tick.tick().await;
            match recover_jobs(
                &state.db,
                None,
                "lease expired (render node stopped responding)",
            )
            .await
            {
                Ok(0) => {}
                Ok(n) => tracing::warn!(jobs = n, "recovered jobs with expired leases"),
                Err(err) => tracing::error!(error = %err.message, "reaper failed"),
            }
        }
    });
}
