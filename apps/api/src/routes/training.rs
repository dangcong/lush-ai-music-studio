//! AI Training (M1): Style LoRA training runs. A run is a job of type `train_lora` on the shared
//! queue (D6); its result is a row in `loras` with status `evaluation`. Admin only for now.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, SqlitePool};

use crate::{
    AppState,
    auth::{Admin, Trainer},
    error::{AppError, AppResult},
    queue::{JOB_SELECT, Job, load_job},
    routes::{lora_review, registry::online_cutoff},
    util::{audit, new_id, now, random_seed},
};

pub const JOB_TYPE: &str = "train_lora";
/// The only trainer M1 knows: Side-Step as vendored in ACE-Step (`train.py fixed`).
pub(crate) const TRAINER: &str = "sidestep";
pub(crate) const TRAINER_ENGINE: &str = "acestep";

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

/// Range checks shared by training runs and LoRA builds.
#[allow(clippy::too_many_arguments)]
pub(crate) fn validate_config(
    rank: i64,
    alpha: i64,
    learning_rate: f64,
    epochs: i64,
    gradient_accumulation: i64,
    save_every: i64,
    warmup_steps: i64,
    max_minutes: i64,
    seed: Option<i64>,
) -> AppResult<()> {
    if seed.is_some_and(|s| !(0..=i64::from(i32::MAX)).contains(&s)) {
        return Err(AppError::validation(
            "Seed phải là số không âm tối đa 2147483647",
        ));
    }
    in_range("rank", rank, 1, 128)?;
    in_range("alpha", alpha, 1, 256)?;
    in_range("epochs", epochs, 1, 1000)?;
    in_range("gradient_accumulation", gradient_accumulation, 1, 32)?;
    in_range("save_every", save_every, 1, epochs)?;
    in_range("warmup_steps", warmup_steps, 0, 10_000)?;
    in_range("max_minutes", max_minutes, 1, 1440)?;
    if !(learning_rate > 0.0 && learning_rate <= 0.01) {
        return Err(AppError::validation(
            "learning_rate phải lớn hơn 0 và không quá 0.01",
        ));
    }
    Ok(())
}

/// One run at a time: every run stops music generation on the node (D21).
pub(crate) async fn ensure_no_active_run(db: &SqlitePool) -> AppResult<()> {
    let busy: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM jobs WHERE type = ? AND status IN ('queued', 'preparing', 'running') LIMIT 1",
    )
    .bind(JOB_TYPE)
    .fetch_optional(db)
    .await?;
    if busy.is_some() {
        return Err(AppError::conflict(
            "Đang có một training run khác chưa kết thúc",
        ));
    }
    Ok(())
}

/// A node must run training and, when `dataset_key` is given, hold that dataset version;
/// otherwise the run would wait forever.
pub(crate) async fn ensure_training_node(
    db: &SqlitePool,
    dataset_key: Option<&str>,
) -> AppResult<()> {
    let node: Option<(String,)> = match dataset_key {
        Some(key) => {
            sqlx::query_as(
                "SELECT n.id FROM node_datasets d JOIN nodes n ON n.id = d.node_id WHERE d.key = ? AND n.status = 'active' AND EXISTS (SELECT 1 FROM json_each(n.job_types) t WHERE t.value = ?) LIMIT 1",
            )
            .bind(key)
            .bind(JOB_TYPE)
            .fetch_optional(db)
            .await?
        }
        None => {
            sqlx::query_as(
                "SELECT n.id FROM nodes n WHERE n.status = 'active' AND EXISTS (SELECT 1 FROM json_each(n.job_types) t WHERE t.value = ?) LIMIT 1",
            )
            .bind(JOB_TYPE)
            .fetch_optional(db)
            .await?
        }
    };
    if node.is_none() {
        return Err(AppError::validation(if dataset_key.is_some() {
            "Không có render node nào hỗ trợ training và có dataset này"
        } else {
            "Không có render node nào hỗ trợ training"
        }));
    }
    Ok(())
}

/// The base model a training run starts from: it must exist, be active and have a trainer.
/// Returns `(engine, key)`.
pub(crate) async fn load_base_model(db: &SqlitePool, id: &str) -> AppResult<(String, String)> {
    let (engine, model_key, model_status): (String, String, String) =
        sqlx::query_as("SELECT engine, key, status FROM models WHERE id = ?")
            .bind(id)
            .fetch_optional(db)
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
    Ok((engine, model_key))
}

/// `Lo-fi Jazz v1` -> `lo-fi-jazz-v1`; the caller appends an id suffix to make it unique.
pub(crate) fn slug(text: &str) -> String {
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
    validate_config(
        req.rank,
        req.alpha,
        req.learning_rate,
        req.epochs,
        req.gradient_accumulation,
        req.save_every,
        req.warmup_steps,
        req.max_minutes,
        req.seed,
    )?;

    let (engine, model_key) = load_base_model(&state.db, &req.base_model_id).await?;
    ensure_training_node(&state.db, Some(req.dataset_key.trim())).await?;
    ensure_no_active_run(&state.db).await?;

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
    Trainer(_user): Trainer,
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
    /// Newest `eval_lora` job of the LoRA; null before one exists.
    eval: Option<Value>,
    /// Listening renders of the LoRA, with their scores.
    renders: Vec<Value>,
    /// Songs kept out of training, playable from the catalog artifact.
    held_out_tracks: Vec<Value>,
    /// The trained Style Preset on this LoRA, once one was made.
    style_preset: Option<Value>,
    /// Newest `lora_files` job of the LoRA (publish or delete).
    files_job: Option<Value>,
    /// Summary of the dataset snapshot the run trained from; null for runs on a node dataset.
    snapshot: Option<Value>,
    /// Kept epochs; the node path never leaves the server.
    checkpoints: Vec<Checkpoint>,
}

/// Columns of [`Checkpoint`], for a query over `lora_checkpoints c`.
pub(crate) const CHECKPOINT_COLUMNS: &str = "c.id, c.epoch, c.step, c.bytes, c.status, c.note, c.expires_at, c.delete_requested_at, c.style_score, c.train_score, c.train_match, c.melody_score, c.melody_match, c.quality_score";

#[derive(Serialize, FromRow)]
pub struct Checkpoint {
    id: String,
    epoch: i64,
    step: Option<i64>,
    bytes: Option<i64>,
    status: String,
    note: String,
    expires_at: Option<String>,
    delete_requested_at: Option<String>,
    style_score: Option<f64>,
    train_score: Option<f64>,
    train_match: Option<String>,
    melody_score: Option<f64>,
    melody_match: Option<String>,
    quality_score: Option<f64>,
}

async fn snapshot_summary(db: &SqlitePool, job_id: &str) -> AppResult<Option<Value>> {
    let row: Option<(String, String, String)> =
        sqlx::query_as("SELECT id, catalog_id, content FROM dataset_snapshots WHERE job_id = ?")
            .bind(job_id)
            .fetch_optional(db)
            .await?;
    let Some((id, catalog_id, content)) = row else {
        return Ok(None);
    };
    let c: Value = serde_json::from_str(&content)?;
    let held_out: Vec<Value> = c["held_out"]
        .as_array()
        .map(|a| a.iter().map(|s| s["filename"].clone()).collect())
        .unwrap_or_default();
    Ok(Some(json!({
        "id": id, "catalog_id": catalog_id, "catalog_name": c["catalog"]["name"],
        "samples": c["samples"].as_array().map_or(0, Vec::len),
        "held_out": held_out, "keyword": c["keyword"], "test_prompt": c["test_prompt"],
        "models": c["models"],
    })))
}

/// Held-out songs of the run's snapshot, playable from their catalog artifact.
async fn held_out_tracks(db: &SqlitePool, job_id: &str) -> AppResult<Vec<Value>> {
    let content: Option<(String,)> =
        sqlx::query_as("SELECT content FROM dataset_snapshots WHERE job_id = ?")
            .bind(job_id)
            .fetch_optional(db)
            .await?;
    let Some((content,)) = content else {
        return Ok(vec![]);
    };
    let c: Value = serde_json::from_str(&content)?;
    Ok(c["held_out"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|s| json!({"filename": s["filename"], "artifact_id": s["artifact_id"]}))
                .collect()
        })
        .unwrap_or_default())
}

#[derive(Default)]
struct ReviewParts {
    eval: Option<Value>,
    renders: Vec<Value>,
    style_preset: Option<Value>,
    files_job: Option<Value>,
}

/// The review state of one LoRA: newest eval job, its renders, the Style made from it, the newest
/// file job. Job payloads are read only for the LoRA id and the operation, never returned.
async fn review_parts(db: &SqlitePool, lora_id: &str) -> AppResult<ReviewParts> {
    #[allow(clippy::type_complexity)]
    let eval: Option<(
        String,
        String,
        f64,
        Option<String>,
        Option<String>,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT id, status, progress, stage, error, result FROM jobs \
             WHERE type = ? AND json_extract(payload, '$.lora.id') = ? \
             ORDER BY created_at DESC, id DESC LIMIT 1",
    )
    .bind(lora_review::JOB_EVAL)
    .bind(lora_id)
    .fetch_optional(db)
    .await?;
    let eval = eval.map(|(job_id, status, progress, stage, error, result)| {
        let result: Value = result
            .and_then(|r| serde_json::from_str(&r).ok())
            .unwrap_or(Value::Null);
        let scorers = json!({
            "mert": result["scorers"]["mert"].as_bool().unwrap_or(false),
            "audiobox": result["scorers"]["audiobox"].as_bool().unwrap_or(false),
        });
        json!({"job_id": job_id, "status": status, "progress": progress, "stage": stage,
               "error": error, "scorers": scorers})
    });

    #[allow(clippy::type_complexity)]
    let rows: Vec<(String, String, Option<String>, Option<String>, String, String, String)> =
        sqlx::query_as(
            "SELECT r.id, r.kind, r.checkpoint_id, r.sample_filename, r.prompt, r.artifact_id, r.scores \
             FROM lora_eval_renders r JOIN artifacts a ON a.id = r.artifact_id \
             WHERE r.lora_id = ? ORDER BY r.created_at, a.kind",
        )
        .bind(lora_id)
        .fetch_all(db)
        .await?;
    let renders = rows
        .into_iter()
        .map(
            |(id, kind, checkpoint_id, sample, prompt, artifact_id, scores)| {
                let scores: Value = serde_json::from_str(&scores).unwrap_or_else(|_| json!({}));
                json!({"id": id, "kind": kind, "checkpoint_id": checkpoint_id,
                   "sample_filename": sample, "prompt": prompt, "artifact_id": artifact_id,
                   "scores": scores})
            },
        )
        .collect();

    let style: Option<(String, String, Option<f64>, bool)> = sqlx::query_as(
        "SELECT id, name, lora_scale, is_active FROM style_presets \
         WHERE kind = 'trained' AND lora_id = ? ORDER BY rowid LIMIT 1",
    )
    .bind(lora_id)
    .fetch_optional(db)
    .await?;
    let style_preset = style.map(|(id, name, lora_scale, is_active)| {
        json!({"id": id, "name": name, "lora_scale": lora_scale, "is_active": is_active})
    });

    let files: Option<(String, String, Option<String>, String)> = sqlx::query_as(
        "SELECT id, status, error, payload FROM jobs \
         WHERE type = 'lora_files' AND json_extract(payload, '$.lora.id') = ? \
         ORDER BY created_at DESC, id DESC LIMIT 1",
    )
    .bind(lora_id)
    .fetch_optional(db)
    .await?;
    let files_job = files.map(|(job_id, status, error, payload)| {
        let payload: Value = serde_json::from_str(&payload).unwrap_or(Value::Null);
        json!({"job_id": job_id, "status": status, "op": payload["op"], "error": error})
    });
    Ok(ReviewParts {
        eval,
        renders,
        style_preset,
        files_job,
    })
}

pub async fn get_training_run(
    State(state): State<AppState>,
    Trainer(_user): Trainer,
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
    #[allow(clippy::type_complexity)]
    let lora: Option<(
        String,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT id, key, name, status, base_key, chosen_checkpoint_id FROM loras WHERE job_id = ?",
    )
    .bind(&id)
    .fetch_optional(&state.db)
    .await?;
    let review = match &lora {
        Some((lora_id, ..)) => review_parts(&state.db, lora_id).await?,
        None => ReviewParts::default(),
    };
    let snapshot = snapshot_summary(&state.db, &id).await?;
    let checkpoints = sqlx::query_as(
        &format!(
            "SELECT {CHECKPOINT_COLUMNS} FROM lora_checkpoints c JOIN loras l ON l.id = c.lora_id WHERE l.job_id = ? ORDER BY c.epoch"
        ),
    )
    .bind(&id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(TrainingRunDetail {
        snapshot,
        checkpoints,
        job,
        log,
        updated_at,
        metrics,
        eval: review.eval,
        renders: review.renders,
        held_out_tracks: held_out_tracks(&state.db, &id).await?,
        style_preset: review.style_preset,
        files_job: review.files_job,
        lora: lora.map(|(id, key, name, status, base_key, chosen)| {
            json!({"id": id, "key": key, "name": name, "status": status,
                   "base_key": base_key.unwrap_or_else(|| key.clone()),
                   "chosen_checkpoint_id": chosen})
        }),
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
