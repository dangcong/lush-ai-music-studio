//! Reference Audio: a captioner listens to an uploaded input and writes a prompt the user can edit
//! (job `describe_input`, spec `docs/superpowers/specs/2026-11-24-reference-auto-prompt-design.md`).

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde_json::{Value, json};

use crate::{
    AppState,
    auth::AuthUser,
    error::{AppError, AppResult},
    routes::{catalogs::normalize_key, inputs::load_audio_input},
    util::{new_id, now},
};

pub const JOB_TYPE: &str = "describe_input";
/// Ahead of queued generations (0), behind `lora_files` (10): the user is waiting on the form.
const PRIORITY: i64 = 5;
/// Preferred when installed: the model that labelled ACE-Step 1.5's own training data.
const PREFERRED_CAPTIONER: &str = "acestep-captioner";
const MAX_CAPTION_CHARS: usize = 4000;

/// `POST /audio-inputs/{id}/prompt-jobs`: queue a `describe_input` job for the caller's input file, or
/// return the one still queued/running for it (200). A new job is 201.
pub async fn create_prompt_job(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let input = load_audio_input(&state, &user, &id)
        .await?
        .ok_or_else(|| AppError::not_found("audio input"))?;

    let live: Option<(String, String)> = sqlx::query_as(
        "SELECT id, json_extract(payload, '$.model.key') FROM jobs \
         WHERE type = ? AND owner_id = ? AND status IN ('queued', 'preparing', 'running') \
           AND json_extract(payload, '$.inputs.audio.artifact_id') = ? \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(JOB_TYPE)
    .bind(&user.id)
    .bind(&input.id)
    .fetch_optional(&state.db)
    .await?;
    if let Some((job_id, model_key)) = live {
        return Ok((
            StatusCode::OK,
            Json(json!({"job_id": job_id, "model_key": model_key})),
        ));
    }

    // Active captioners installed on an active node; the preferred one first.
    let installed: Vec<(String,)> = sqlx::query_as(
        "SELECT DISTINCT m.key FROM models m \
           JOIN model_installations mi ON mi.model_id = m.id \
           JOIN nodes n ON n.id = mi.node_id \
         WHERE m.kind = 'captioner' AND m.status = 'active' AND n.status = 'active' \
         ORDER BY m.key",
    )
    .fetch_all(&state.db)
    .await?;
    let model_key = installed
        .iter()
        .map(|(k,)| k.as_str())
        .find(|k| *k == PREFERRED_CAPTIONER)
        .or_else(|| installed.first().map(|(k,)| k.as_str()))
        .ok_or_else(|| AppError::validation("Chưa có model mô tả trên máy render"))?
        .to_string();

    let job_id = new_id();
    let ts = now();
    let payload = json!({"model": {"key": model_key}, "inputs": {"audio": input.payload()}});
    sqlx::query(
        "INSERT INTO jobs (id, type, priority, owner_id, title, payload, max_attempts, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, 2, ?, ?)",
    )
    .bind(&job_id)
    .bind(JOB_TYPE)
    .bind(PRIORITY)
    .bind(&user.id)
    .bind(format!("Viết prompt: {}", input.filename))
    .bind(payload.to_string())
    .bind(&ts)
    .bind(&ts)
    .execute(&state.db)
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"job_id": job_id, "model_key": model_key})),
    ))
}

/// Check and tidy a worker's `describe_input` result before it is stored: a non-empty caption (trimmed,
/// at most 4000 characters), the key in ACE-Step's keyscale form, a tempo in 20–300 BPM (else null).
pub fn clean_result(result: &Value) -> AppResult<Value> {
    let text = |k: &str| result[k].as_str().unwrap_or_default().trim().to_string();
    let caption = text("caption");
    if caption.is_empty() || caption.chars().count() > MAX_CAPTION_CHARS {
        return Err(AppError::validation("result.caption phải có 1–4000 ký tự"));
    }
    let tempo = result["tempo_bpm"]
        .as_f64()
        .filter(|t| t.is_finite() && (20.0..=300.0).contains(t));
    let key = result["key"].as_str().and_then(normalize_key);
    Ok(json!({
        "caption": caption, "mood": text("mood"), "instruments": text("instruments"),
        "vocals": text("vocals"), "source_ref": text("source_ref"),
        "tempo_bpm": tempo, "key": key,
    }))
}
