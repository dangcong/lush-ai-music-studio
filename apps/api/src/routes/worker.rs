//! Worker protocol v1 (render node -> control plane). Contract: docs/WORKER_PROTOCOL.md.

use std::path::PathBuf;

use axum::{
    Json,
    extract::{Multipart, Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::types::Json as DbJson;
use tokio::{fs, io::AsyncWriteExt};

use crate::{
    AppState,
    auth::NodeAuth,
    error::{AppError, AppResult},
    queue::recover_jobs,
    util::{audit, new_id, new_token, now, now_plus},
};

pub const PROTOCOL_VERSION: i64 = 1;
const MAX_LOG_BYTES: i64 = 64 * 1024;

pub(crate) fn lease_lost() -> AppError {
    AppError::new(
        StatusCode::CONFLICT,
        "lease_lost",
        "Job không còn thuộc về node này",
    )
}

// ---------- register / heartbeat ----------

#[derive(Deserialize)]
pub struct ModelReport {
    engine: String,
    key: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    capabilities: Vec<String>,
}

#[derive(Deserialize)]
pub struct RegisterRequest {
    protocol_version: i64,
    #[serde(default)]
    worker_version: String,
    #[serde(default = "default_true")]
    fresh_start: bool,
    #[serde(default)]
    job_types: Vec<String>,
    #[serde(default)]
    engines: Value,
    #[serde(default)]
    models: Vec<ModelReport>,
    #[serde(default)]
    loras: Vec<String>,
    #[serde(default)]
    gpu: Value,
}

fn default_true() -> bool {
    true
}

pub async fn register(
    State(state): State<AppState>,
    node: NodeAuth,
    Json(req): Json<RegisterRequest>,
) -> AppResult<Json<Value>> {
    if req.protocol_version != PROTOCOL_VERSION {
        return Err(AppError::new(
            StatusCode::CONFLICT,
            "protocol_mismatch",
            format!(
                "Control plane dùng protocol {PROTOCOL_VERSION}, worker gửi {}",
                req.protocol_version
            ),
        ));
    }
    let now = now();
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "UPDATE nodes SET worker_version = ?, protocol_version = ?, job_types = ?, engines = ?, gpu = ?, \
         last_seen = ?, registered_at = ? WHERE id = ?",
    )
    .bind(&req.worker_version)
    .bind(req.protocol_version)
    .bind(DbJson(&req.job_types))
    .bind(if req.engines.is_array() { req.engines.to_string() } else { "[]".into() })
    .bind(if req.gpu.is_object() { req.gpu.to_string() } else { "{}".into() })
    .bind(&now)
    .bind(&now)
    .bind(&node.id)
    .execute(&mut *tx)
    .await?;

    // The node's installation list is replaced by what it reports. Unknown models join the registry;
    // unknown LoRAs are ignored (LoRAs only enter the registry through training).
    sqlx::query("DELETE FROM model_installations WHERE node_id = ?")
        .bind(&node.id)
        .execute(&mut *tx)
        .await?;
    for model in req
        .models
        .iter()
        .filter(|m| !m.engine.trim().is_empty() && !m.key.trim().is_empty())
    {
        sqlx::query(
            "INSERT INTO models (id, engine, key, name, version, capabilities, created_at) VALUES (?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT (engine, key) DO UPDATE SET capabilities = excluded.capabilities",
        )
        .bind(new_id())
        .bind(model.engine.trim())
        .bind(model.key.trim())
        .bind(model.name.as_deref().filter(|n| !n.trim().is_empty()).unwrap_or(&model.key))
        .bind(model.version.as_deref().unwrap_or(""))
        .bind(DbJson(&model.capabilities))
        .bind(&now)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT OR IGNORE INTO model_installations (node_id, model_id) \
             SELECT ?, id FROM models WHERE engine = ? AND key = ?",
        )
        .bind(&node.id)
        .bind(model.engine.trim())
        .bind(model.key.trim())
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query("DELETE FROM lora_installations WHERE node_id = ?")
        .bind(&node.id)
        .execute(&mut *tx)
        .await?;
    for key in &req.loras {
        sqlx::query("INSERT OR IGNORE INTO lora_installations (node_id, lora_id) SELECT ?, id FROM loras WHERE key = ?")
            .bind(&node.id)
            .bind(key)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;

    if req.fresh_start {
        let recovered = recover_jobs(
            &state.db,
            Some(&node.id),
            "render node restarted while holding this job",
        )
        .await?;
        audit(
            &state.db,
            None,
            "node.registered",
            "node",
            &node.id,
            json!({"worker_version": req.worker_version, "recovered_jobs": recovered}),
        )
        .await;
        tracing::info!(node = %node.name, recovered, "render node registered");
    }
    Ok(Json(json!({
        "node_id": node.id,
        "name": node.name,
        "heartbeat_interval": state.cfg.heartbeat_interval,
        "lease_seconds": state.cfg.lease_seconds,
        "poll_interval": state.cfg.poll_interval,
    })))
}

#[derive(Deserialize)]
pub struct HeartbeatRequest {
    #[serde(default)]
    current_job_id: Option<String>,
    #[serde(default)]
    gpu: Value,
}

pub async fn heartbeat(
    State(state): State<AppState>,
    node: NodeAuth,
    Json(req): Json<HeartbeatRequest>,
) -> AppResult<Json<Value>> {
    // Live GPU readings are merged over the static info sent at registration.
    let gpu_patch = if req.gpu.is_object() {
        req.gpu.to_string()
    } else {
        "{}".into()
    };
    let (maintenance,): (bool,) = sqlx::query_as(
        "UPDATE nodes SET last_seen = ?, gpu = json_patch(gpu, ?) WHERE id = ? RETURNING maintenance",
    )
    .bind(now())
    .bind(gpu_patch)
    .bind(&node.id)
    .fetch_one(&state.db)
    .await?;
    if let Some(job_id) = &req.current_job_id {
        sqlx::query("UPDATE jobs SET lease_expires_at = ? WHERE id = ? AND node_id = ? AND status IN ('preparing', 'running')")
            .bind(now_plus(state.cfg.lease_seconds))
            .bind(job_id)
            .bind(&node.id)
            .execute(&state.db)
            .await?;
    }
    let cancel: Vec<(String,)> = sqlx::query_as(
        "SELECT id FROM jobs WHERE node_id = ? AND status IN ('preparing', 'running') AND cancel_requested = 1",
    )
    .bind(&node.id)
    .fetch_all(&state.db)
    .await?;
    let cancel_job_ids: Vec<String> = cancel.into_iter().map(|(id,)| id).collect();
    Ok(Json(
        json!({"cancel_job_ids": cancel_job_ids, "maintenance": maintenance}),
    ))
}

// ---------- claim ----------

/// Capability-based scheduling in one atomic statement: the oldest, highest-priority queued job this
/// node is able to run, provided the node is idle (V1: one GPU job per node at a time).
pub async fn claim(State(state): State<AppState>, node: NodeAuth) -> AppResult<Response> {
    let now = now();
    sqlx::query("UPDATE nodes SET last_seen = ? WHERE id = ?")
        .bind(&now)
        .bind(&node.id)
        .execute(&state.db)
        .await?;
    let row: Option<(String, String, i64, String, String, String)> = sqlx::query_as(
        "UPDATE jobs SET status = 'preparing', node_id = ?1, lease_token = ?2, lease_expires_at = ?3, \
           attempt = attempt + 1, started_at = COALESCE(started_at, ?4), updated_at = ?4, \
           progress = 0, stage = 'preparing', message = NULL, error = NULL \
         WHERE id = ( \
           SELECT j.id FROM jobs j \
           WHERE j.status = 'queued' \
             AND EXISTS (SELECT 1 FROM nodes n WHERE n.id = ?1 AND n.status = 'active' AND n.maintenance = 0 \
                         AND EXISTS (SELECT 1 FROM json_each(n.job_types) t WHERE t.value = j.type)) \
             AND NOT EXISTS (SELECT 1 FROM jobs busy WHERE busy.node_id = ?1 AND busy.status IN ('preparing', 'running')) \
             AND (j.model_id IS NULL OR EXISTS (SELECT 1 FROM model_installations mi WHERE mi.node_id = ?1 AND mi.model_id = j.model_id)) \
             AND (j.lora_id IS NULL OR EXISTS (SELECT 1 FROM lora_installations li WHERE li.node_id = ?1 AND li.lora_id = j.lora_id)) \
           ORDER BY j.priority DESC, j.created_at ASC, j.id ASC LIMIT 1) \
         RETURNING id, type, attempt, payload, lease_token, lease_expires_at",
    )
    .bind(&node.id)
    .bind(new_token())
    .bind(now_plus(state.cfg.lease_seconds))
    .bind(&now)
    .fetch_optional(&state.db)
    .await?;
    let Some((id, job_type, attempt, payload, lease_token, lease_expires_at)) = row else {
        return Ok(StatusCode::NO_CONTENT.into_response());
    };
    tracing::info!(node = %node.name, job = %id, attempt, "job claimed");
    let payload: Value = serde_json::from_str(&payload)?;
    Ok(Json(json!({
        "id": id, "type": job_type, "attempt": attempt, "payload": payload,
        "lease_token": lease_token, "lease_expires_at": lease_expires_at,
    }))
    .into_response())
}

// ---------- progress ----------

#[derive(Deserialize)]
pub struct ProgressRequest {
    lease_token: String,
    progress: f64,
    #[serde(default)]
    stage: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

pub async fn progress(
    State(state): State<AppState>,
    node: NodeAuth,
    Path(job_id): Path<String>,
    Json(req): Json<ProgressRequest>,
) -> AppResult<Json<Value>> {
    let message = req
        .message
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty());
    let running = req.stage.as_deref() == Some("running");
    let row: Option<(bool,)> = sqlx::query_as(
        "UPDATE jobs SET progress = ?1, \
           status = CASE WHEN ?2 THEN 'running' ELSE status END, \
           stage = CASE WHEN ?2 THEN 'running' ELSE stage END, \
           message = COALESCE(?3, message), \
           log = CASE WHEN ?3 IS NULL THEN log ELSE substr(log || ?3 || char(10), -?4) END, \
           lease_expires_at = ?5, updated_at = ?6 \
         WHERE id = ?7 AND node_id = ?8 AND lease_token = ?9 AND status IN ('preparing', 'running') \
         RETURNING cancel_requested",
    )
    .bind(req.progress.clamp(0.0, 1.0))
    .bind(running)
    .bind(message)
    .bind(MAX_LOG_BYTES)
    .bind(now_plus(state.cfg.lease_seconds))
    .bind(now())
    .bind(&job_id)
    .bind(&node.id)
    .bind(&req.lease_token)
    .fetch_optional(&state.db)
    .await?;
    let (cancel_requested,) = row.ok_or_else(lease_lost)?;
    Ok(Json(json!({"cancel_requested": cancel_requested})))
}

// ---------- artifacts ----------

/// Allowed upload kinds and their file extensions. The client file name is only used for its extension.
fn artifact_type(kind: &str, file_name: &str) -> Option<(&'static str, &'static str)> {
    let ext = file_name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match (kind, ext.as_str()) {
        ("audio_master", "wav") => Some(("wav", "audio/wav")),
        ("audio_master", "flac") => Some(("flac", "audio/flac")),
        ("audio_preview", "mp3") => Some(("mp3", "audio/mpeg")),
        _ => None,
    }
}

/// Removes a temporary upload unless it was moved into place.
pub(crate) struct TempFile(pub(crate) Option<PathBuf>);

impl Drop for TempFile {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

pub async fn upload_artifact(
    State(state): State<AppState>,
    node: NodeAuth,
    Path(job_id): Path<String>,
    mut multipart: Multipart,
) -> AppResult<Json<Value>> {
    let (mut lease_token, mut kind, mut expected_sha) = (None, None, None);
    let mut upload: Option<(TempFile, String, String, i64)> = None; // (file, client name, sha256, size)
    let tmp_dir = state.cfg.data_dir.join("tmp");
    fs::create_dir_all(&tmp_dir).await?;

    while let Some(mut field) = multipart.next_field().await? {
        match field.name().unwrap_or_default() {
            "lease_token" => lease_token = Some(field.text().await?),
            "kind" => kind = Some(field.text().await?),
            "sha256" => expected_sha = Some(field.text().await?.trim().to_ascii_lowercase()),
            "file" => {
                let client_name = field.file_name().unwrap_or_default().to_string();
                let path = tmp_dir.join(new_id());
                let guard = TempFile(Some(path.clone()));
                let mut file = fs::File::create(&path).await?;
                let (mut hasher, mut size) = (Sha256::new(), 0i64);
                while let Some(chunk) = field.chunk().await? {
                    hasher.update(&chunk);
                    size += chunk.len() as i64;
                    file.write_all(&chunk).await?;
                }
                file.flush().await?;
                upload = Some((guard, client_name, hex::encode(hasher.finalize()), size));
            }
            _ => {}
        }
    }
    let missing = |name: &str| AppError::bad_request(format!("Thiếu field '{name}'"));
    let lease_token = lease_token.ok_or_else(|| missing("lease_token"))?;
    let kind = kind.ok_or_else(|| missing("kind"))?;
    let expected_sha = expected_sha.ok_or_else(|| missing("sha256"))?;
    let (mut temp, client_name, actual_sha, size) = upload.ok_or_else(|| missing("file"))?;

    let held: Option<(i64,)> = sqlx::query_as(
        "SELECT 1 FROM jobs WHERE id = ? AND node_id = ? AND lease_token = ? AND status IN ('preparing', 'running')",
    )
    .bind(&job_id)
    .bind(&node.id)
    .bind(&lease_token)
    .fetch_optional(&state.db)
    .await?;
    held.ok_or_else(lease_lost)?;
    let (ext, content_type) = artifact_type(&kind, &client_name).ok_or_else(|| {
        AppError::validation("Loại artifact hoặc định dạng file không được hỗ trợ")
    })?;
    if size == 0 {
        return Err(AppError::validation("File rỗng"));
    }
    if actual_sha != expected_sha {
        return Err(AppError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "checksum_mismatch",
            "SHA-256 của file không khớp",
        ));
    }

    // Storage keys are built only from server-side ids, never from client input.
    let storage_key = format!("artifacts/{job_id}/{kind}.{ext}");
    let final_path = state.cfg.data_dir.join(&storage_key);
    fs::create_dir_all(final_path.parent().expect("storage key has a parent")).await?;
    let previous: Option<(String, String)> =
        sqlx::query_as("SELECT id, storage_key FROM artifacts WHERE job_id = ? AND kind = ?")
            .bind(&job_id)
            .bind(&kind)
            .fetch_optional(&state.db)
            .await?;
    if let Some((_, old_key)) = previous
        .as_ref()
        .filter(|(_, old_key)| *old_key != storage_key)
    {
        let _ = fs::remove_file(state.cfg.data_dir.join(old_key)).await;
    }
    let source = temp.0.take().expect("temp file present");
    if let Err(err) = fs::rename(&source, &final_path).await {
        temp.0 = Some(source);
        return Err(err.into());
    }

    let artifact_id = previous.map(|(id, _)| id).unwrap_or_else(new_id);
    let short_job = job_id
        .chars()
        .rev()
        .take(8)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    sqlx::query(
        "INSERT INTO artifacts (id, kind, job_id, storage_key, filename, content_type, size_bytes, sha256, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT (job_id, kind) DO UPDATE SET storage_key = excluded.storage_key, filename = excluded.filename, \
         content_type = excluded.content_type, size_bytes = excluded.size_bytes, sha256 = excluded.sha256",
    )
    .bind(&artifact_id)
    .bind(&kind)
    .bind(&job_id)
    .bind(&storage_key)
    .bind(format!("lush-{short_job}-{}.{ext}", kind.trim_start_matches("audio_")))
    .bind(content_type)
    .bind(size)
    .bind(&actual_sha)
    .bind(now())
    .execute(&state.db)
    .await?;
    Ok(Json(json!({"artifact_id": artifact_id})))
}

// ---------- complete / fail / release ----------

#[derive(Deserialize)]
pub struct CompleteRequest {
    lease_token: String,
    #[serde(default)]
    result: Value,
}

pub async fn complete(
    State(state): State<AppState>,
    node: NodeAuth,
    Path(job_id): Path<String>,
    Json(req): Json<CompleteRequest>,
) -> AppResult<Json<Value>> {
    let mut tx = state.db.begin().await?;
    #[allow(clippy::type_complexity)]
    let row: Option<(String, String, String, Option<String>, String, String, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT type, status, owner_id, project_id, title, request, payload, model_id, lora_id FROM jobs \
         WHERE id = ? AND node_id = ? AND lease_token = ?",
    )
    .bind(&job_id)
    .bind(&node.id)
    .bind(&req.lease_token)
    .fetch_optional(&mut *tx)
    .await?;
    let (job_type, status, owner_id, project_id, title, request, payload, model_id, lora_id) =
        row.ok_or_else(lease_lost)?;
    if status == "completed" {
        return Ok(Json(json!({"ok": true}))); // idempotent repeat
    }
    if status != "preparing" && status != "running" {
        return Err(lease_lost());
    }
    let now = now();
    let result = if req.result.is_object() {
        req.result
    } else {
        json!({})
    };

    if job_type == "generate" {
        let master: Option<(String,)> =
            sqlx::query_as("SELECT id FROM artifacts WHERE job_id = ? AND kind = 'audio_master'")
                .bind(&job_id)
                .fetch_optional(&mut *tx)
                .await?;
        if master.is_none() {
            return Err(AppError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "missing_artifact",
                "Job generate chưa có artifact audio_master",
            ));
        }
        let request: Value = serde_json::from_str(&request)?;
        let payload: Value = serde_json::from_str(&payload)?;
        let mut params = payload["params"].as_object().cloned().unwrap_or_default();
        // How the track was made: generation mode and the input audio it used.
        if let Some(mode) = payload["mode"].as_str().filter(|m| *m != "text") {
            params.insert("mode".into(), json!(mode));
            params.insert("inputs".into(), payload["inputs"].clone());
        }
        for key in ["engine", "model_key", "sample_rate", "engine_info"] {
            if let Some(value) = result.get(key).filter(|v| !v.is_null()) {
                params.insert(key.into(), value.clone());
            }
        }
        let track_id = new_id();
        sqlx::query(
            "INSERT INTO tracks (id, title, owner_id, project_id, job_id, prompt, prompt_final, genre_id, subgenre_id, \
             preset_id, model_id, lora_id, lora_scale, seed, bpm, key, time_signature, duration, params, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&track_id)
        .bind(&title)
        .bind(&owner_id)
        .bind(&project_id)
        .bind(&job_id)
        .bind(request["prompt"].as_str().unwrap_or_default())
        .bind(payload["prompt"].as_str().unwrap_or_default())
        .bind(request["genre_id"].as_str().filter(|s| !s.is_empty()))
        .bind(request["subgenre_id"].as_str().filter(|s| !s.is_empty()))
        .bind(request["preset_id"].as_str().filter(|s| !s.is_empty()))
        .bind(&model_id)
        .bind(&lora_id)
        .bind(payload["lora"]["scale"].as_f64())
        .bind(result["seed"].as_i64().or(payload["seed"].as_i64()))
        .bind(payload["bpm"].as_i64())
        .bind(payload["key"].as_str())
        .bind(payload["time_signature"].as_str())
        .bind(result["duration"].as_f64().or(payload["duration"].as_f64()))
        .bind(Value::Object(params).to_string())
        .bind(&now)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE artifacts SET track_id = ? WHERE job_id = ?")
            .bind(&track_id)
            .bind(&job_id)
            .execute(&mut *tx)
            .await?;
    }

    sqlx::query(
        "UPDATE jobs SET status = 'completed', progress = 1, stage = NULL, message = NULL, result = ?, error = NULL, \
         lease_expires_at = NULL, finished_at = ?, updated_at = ? WHERE id = ?",
    )
    .bind(result.to_string())
    .bind(&now)
    .bind(&now)
    .bind(&job_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    tracing::info!(node = %node.name, job = %job_id, "job completed");
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
pub struct FailRequest {
    lease_token: String,
    #[serde(default)]
    error: String,
    #[serde(default)]
    retryable: bool,
    #[serde(default)]
    cancelled: bool,
}

pub async fn fail(
    State(state): State<AppState>,
    node: NodeAuth,
    Path(job_id): Path<String>,
    Json(req): Json<FailRequest>,
) -> AppResult<Json<Value>> {
    let now = now();
    // ?1 cancelled, ?2 retryable. A user cancel always wins; a retry needs attempts left.
    let row: Option<(String,)> = sqlx::query_as(
        "UPDATE jobs SET \
           status = CASE WHEN ?1 OR cancel_requested = 1 THEN 'cancelled' \
                         WHEN ?2 AND attempt < max_attempts THEN 'queued' ELSE 'failed' END, \
           error = CASE WHEN ?1 OR cancel_requested = 1 THEN NULL ELSE ?3 END, \
           node_id = CASE WHEN NOT (?1 OR cancel_requested = 1) AND ?2 AND attempt < max_attempts THEN NULL ELSE node_id END, \
           finished_at = CASE WHEN NOT (?1 OR cancel_requested = 1) AND ?2 AND attempt < max_attempts THEN NULL ELSE ?4 END, \
           log = log || '[worker] ' || ?3 || char(10), \
           lease_token = NULL, lease_expires_at = NULL, stage = NULL, updated_at = ?4 \
         WHERE id = ?5 AND node_id = ?6 AND lease_token = ?7 AND status IN ('preparing', 'running') \
         RETURNING status",
    )
    .bind(req.cancelled)
    .bind(req.retryable)
    .bind(if req.error.trim().is_empty() { "unknown error" } else { req.error.trim() })
    .bind(&now)
    .bind(&job_id)
    .bind(&node.id)
    .bind(&req.lease_token)
    .fetch_optional(&state.db)
    .await?;
    let (status,) = row.ok_or_else(lease_lost)?;
    tracing::warn!(node = %node.name, job = %job_id, %status, error = %req.error, "job reported as failed");
    Ok(Json(json!({"status": status})))
}

#[derive(Deserialize)]
pub struct ReleaseRequest {
    lease_token: String,
}

/// Graceful worker shutdown: hand the job back without spending an attempt.
pub async fn release(
    State(state): State<AppState>,
    node: NodeAuth,
    Path(job_id): Path<String>,
    Json(req): Json<ReleaseRequest>,
) -> AppResult<Json<Value>> {
    let now = now();
    let row: Option<(String,)> = sqlx::query_as(
        "UPDATE jobs SET \
           status = CASE WHEN cancel_requested = 1 THEN 'cancelled' ELSE 'queued' END, \
           finished_at = CASE WHEN cancel_requested = 1 THEN ?1 ELSE NULL END, \
           node_id = CASE WHEN cancel_requested = 1 THEN node_id ELSE NULL END, \
           attempt = MAX(attempt - 1, 0), progress = 0, stage = NULL, message = NULL, \
           log = log || '[worker] released by render node' || char(10), \
           lease_token = NULL, lease_expires_at = NULL, updated_at = ?1 \
         WHERE id = ?2 AND node_id = ?3 AND lease_token = ?4 AND status IN ('preparing', 'running') \
         RETURNING status",
    )
    .bind(&now)
    .bind(&job_id)
    .bind(&node.id)
    .bind(&req.lease_token)
    .fetch_optional(&state.db)
    .await?;
    let (status,) = row.ok_or_else(lease_lost)?;
    Ok(Json(json!({"status": status})))
}
