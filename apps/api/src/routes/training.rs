//! AI Training (M1): Style LoRA training runs. A run is a job of type `train_lora` on the shared
//! queue (D6); its result is a row in `loras` with status `evaluation`. Admin only for now.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::FromRow;

use crate::{
    AppState,
    auth::Admin,
    error::{AppError, AppResult},
    queue::{JOB_SELECT, Job, load_job},
    routes::registry::online_cutoff,
    util::{audit, new_id, now, random_seed},
};

pub const JOB_TYPE: &str = "train_lora";
/// The only trainer M1 knows: Side-Step as vendored in ACE-Step (`train.py fixed`).
const TRAINER: &str = "sidestep";
const TRAINER_ENGINE: &str = "acestep";

#[derive(Serialize, Deserialize)]
pub struct TrainingRunRequest {
    pub name: String,
    pub dataset_key: String,
    pub base_model_id: String,
    pub rank: i64,
    pub alpha: i64,
    pub learning_rate: f64,
    pub epochs: i64,
    pub gradient_accumulation: i64,
    pub save_every: i64,
    pub warmup_steps: i64,
    #[serde(default)]
    pub seed: Option<i64>,
    /// Wall-clock cap for the whole run (preprocess + training). The worker enforces it.
    pub max_minutes: i64,
}

fn in_range(name: &str, value: i64, min: i64, max: i64) -> AppResult<()> {
    if (min..=max).contains(&value) {
        Ok(())
    } else {
        Err(AppError::validation(format!(
            "{name} phải trong khoảng {min}–{max}"
        )))
    }
}

/// `Lo-fi Jazz v1` -> `lo-fi-jazz-v1`; the caller appends an id suffix to make it unique.
fn slug(text: &str) -> String {
    let mut out = String::new();
    for ch in text.trim().to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').chars().take(40).collect::<String>();
    if out.is_empty() { "lora".into() } else { out }
}

pub async fn create_training_run(
    State(state): State<AppState>,
    Admin(admin): Admin,
    Json(req): Json<TrainingRunRequest>,
) -> AppResult<(StatusCode, Json<Job>)> {
    let name = req.name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(AppError::validation("Tên LoRA phải có 1–80 ký tự"));
    }
    in_range("rank", req.rank, 1, 128)?;
    in_range("alpha", req.alpha, 1, 256)?;
    in_range("epochs", req.epochs, 1, 1000)?;
    in_range("gradient_accumulation", req.gradient_accumulation, 1, 32)?;
    in_range("save_every", req.save_every, 1, req.epochs)?;
    in_range("warmup_steps", req.warmup_steps, 0, 10_000)?;
    in_range("max_minutes", req.max_minutes, 1, 1440)?;
    if !(req.learning_rate > 0.0 && req.learning_rate <= 0.01) {
        return Err(AppError::validation(
            "learning_rate phải lớn hơn 0 và không quá 0.01",
        ));
    }

    let (engine, model_key, model_status): (String, String, String) =
        sqlx::query_as("SELECT engine, key, status FROM models WHERE id = ?")
            .bind(&req.base_model_id)
            .fetch_optional(&state.db)
            .await?
            .ok_or_else(|| AppError::validation("Base model không tồn tại"))?;
    if model_status != "active" {
        return Err(AppError::validation("Base model đang bị tắt"));
    }
    if engine != TRAINER_ENGINE {
        return Err(AppError::validation(
            "Chưa có trainer cho engine của model này",
        ));
    }
    // A node must both run training and hold this dataset version, otherwise the run would wait forever.
    let node: Option<(String,)> = sqlx::query_as(
        "SELECT n.id FROM node_datasets d JOIN nodes n ON n.id = d.node_id \
         WHERE d.key = ? AND n.status = 'active' \
           AND EXISTS (SELECT 1 FROM json_each(n.job_types) t WHERE t.value = ?) LIMIT 1",
    )
    .bind(req.dataset_key.trim())
    .bind(JOB_TYPE)
    .fetch_optional(&state.db)
    .await?;
    if node.is_none() {
        return Err(AppError::validation(
            "Không có render node nào hỗ trợ training và có dataset này",
        ));
    }
    // One run at a time: every run stops music generation on the node (D21).
    let busy: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM jobs WHERE type = ? AND status IN ('queued', 'preparing', 'running') LIMIT 1",
    )
    .bind(JOB_TYPE)
    .fetch_optional(&state.db)
    .await?;
    if busy.is_some() {
        return Err(AppError::conflict(
            "Đang có một training run khác chưa kết thúc",
        ));
    }

    let (id, now) = (new_id(), now());
    let seed = req.seed.unwrap_or_else(random_seed);
    let lora_key = format!("{}-{}", slug(name), &id[id.len() - 8..]);
    let payload = json!({
        "lora": {"key": lora_key, "name": name},
        "dataset": {"key": req.dataset_key.trim()},
        "base_model": {"id": req.base_model_id, "engine": engine, "key": model_key},
        "trainer": TRAINER,
        "config": {
            "rank": req.rank, "alpha": req.alpha, "learning_rate": req.learning_rate,
            "epochs": req.epochs, "gradient_accumulation": req.gradient_accumulation,
            "save_every": req.save_every, "warmup_steps": req.warmup_steps, "seed": seed,
        },
        "limits": {"max_seconds": req.max_minutes * 60},
    });
    // max_attempts = 1: a training run is never restarted on its own.
    sqlx::query(
        "INSERT INTO jobs (id, type, owner_id, title, request, payload, max_attempts, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, 1, ?, ?)",
    )
    .bind(&id)
    .bind(JOB_TYPE)
    .bind(&admin.id)
    .bind(name)
    .bind(serde_json::to_value(&req)?.to_string())
    .bind(payload.to_string())
    .bind(&now)
    .bind(&now)
    .execute(&state.db)
    .await?;
    audit(
        &state.db,
        Some(&admin.id),
        "training.created",
        "job",
        &id,
        json!({"name": name, "dataset_key": req.dataset_key.trim(), "lora_key": lora_key}),
    )
    .await;
    let job = load_job(&state.db, &id)
        .await?
        .ok_or_else(|| AppError::not_found("job"))?;
    Ok((StatusCode::CREATED, Json(job)))
}

pub async fn list_training_runs(
    State(state): State<AppState>,
    Admin(_admin): Admin,
) -> AppResult<Json<Vec<Job>>> {
    let jobs = sqlx::query_as(&format!(
        "{JOB_SELECT} WHERE j.type = ? ORDER BY j.created_at DESC, j.id DESC LIMIT 100"
    ))
    .bind(JOB_TYPE)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(jobs))
}

#[derive(Serialize, FromRow)]
pub struct Metric {
    sequence: i64,
    global_step: Option<i64>,
    epoch: Option<i64>,
    train_loss: Option<f64>,
    learning_rate: Option<f64>,
    elapsed_seconds: Option<f64>,
    gpu_memory_mb: Option<i64>,
    recorded_at: String,
}

#[derive(Serialize)]
pub struct TrainingRunDetail {
    #[serde(flatten)]
    job: Job,
    log: String,
    /// When the worker last reported anything for this run.
    updated_at: String,
    metrics: Vec<Metric>,
    lora: Option<Value>,
}

pub async fn get_training_run(
    State(state): State<AppState>,
    Admin(_admin): Admin,
    Path(id): Path<String>,
) -> AppResult<Json<TrainingRunDetail>> {
    let job = load_job(&state.db, &id)
        .await?
        .filter(|job| job.job_type == JOB_TYPE)
        .ok_or_else(|| AppError::not_found("training run"))?;
    let (log, updated_at): (String, String) =
        sqlx::query_as("SELECT log, updated_at FROM jobs WHERE id = ?")
            .bind(&id)
            .fetch_one(&state.db)
            .await?;
    let metrics = sqlx::query_as(
        "SELECT sequence, global_step, epoch, train_loss, learning_rate, elapsed_seconds, gpu_memory_mb, recorded_at \
         FROM training_metrics WHERE job_id = ? ORDER BY sequence LIMIT 5000",
    )
    .bind(&id)
    .fetch_all(&state.db)
    .await?;
    let lora: Option<(String, String, String, String)> =
        sqlx::query_as("SELECT id, key, name, status FROM loras WHERE job_id = ?")
            .bind(&id)
            .fetch_optional(&state.db)
            .await?;
    Ok(Json(TrainingRunDetail {
        job,
        log,
        updated_at,
        metrics,
        lora: lora.map(
            |(id, key, name, status)| json!({"id": id, "key": key, "name": name, "status": status}),
        ),
    }))
}

#[derive(Serialize, FromRow)]
pub struct NodeDataset {
    key: String,
    node_id: String,
    node_name: String,
    online: bool,
    can_train: bool,
    info: sqlx::types::Json<Value>,
}

pub async fn list_datasets(
    State(state): State<AppState>,
    Admin(_admin): Admin,
) -> AppResult<Json<Vec<NodeDataset>>> {
    let rows = sqlx::query_as(
        "SELECT d.key, d.node_id, n.name AS node_name, d.info, \
           (n.status = 'active' AND n.last_seen IS NOT NULL AND n.last_seen > ?) AS online, \
           EXISTS (SELECT 1 FROM json_each(n.job_types) t WHERE t.value = ?) AS can_train \
         FROM node_datasets d JOIN nodes n ON n.id = d.node_id ORDER BY d.key, n.name",
    )
    .bind(online_cutoff(&state))
    .bind(JOB_TYPE)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows))
}

/// The training run that currently holds a GPU, for the "node is training" notice on the website.
pub async fn active_training(state: &AppState) -> AppResult<Option<Value>> {
    #[allow(clippy::type_complexity)]
    let row: Option<(String, String, Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT j.id, j.title, j.stage, j.started_at, n.name FROM jobs j LEFT JOIN nodes n ON n.id = j.node_id \
         WHERE j.type = ? AND j.status IN ('preparing', 'running') ORDER BY j.started_at LIMIT 1",
    )
    .bind(JOB_TYPE)
    .fetch_optional(&state.db)
    .await?;
    Ok(row.map(|(id, title, stage, started_at, node_name)| {
        json!({"job_id": id, "title": title, "stage": stage, "started_at": started_at, "node_name": node_name})
    }))
}
