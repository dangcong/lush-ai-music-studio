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
    routes::{catalogs::normalize_key, inputs::load_audio_input, jobs::cut_text},
    util::{new_id, now},
};

pub const JOB_TYPE: &str = "describe_input";
/// Ahead of queued generations (0), behind `lora_files` (10): the user is waiting on the form.
const PRIORITY: i64 = 5;
/// Preferred when installed: the model that labelled ACE-Step 1.5's own training data.
const PREFERRED_CAPTIONER: &str = "acestep-captioner";
/// The caption becomes the Create form's prompt, whose limit is 2000 characters.
const MAX_CAPTION_CHARS: usize = 2000;

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

    // Active captioners installed on an active node; the preferred one first.
    let installed: Vec<(String, String)> = sqlx::query_as(
        "SELECT DISTINCT m.id, m.key FROM models m \
           JOIN model_installations mi ON mi.model_id = m.id \
           JOIN nodes n ON n.id = mi.node_id \
         WHERE m.kind = 'captioner' AND m.status = 'active' AND n.status = 'active' \
         ORDER BY m.key",
    )
    .fetch_all(&state.db)
    .await?;
    let chosen = installed
        .iter()
        .find(|(_, k)| k == PREFERRED_CAPTIONER)
        .or_else(|| installed.first());

    // One statement: a live job for this input and owner wins, so two quick calls never make two jobs.
    let job_id = new_id();
    let ts = now();
    if let Some((model_id, model_key)) = chosen {
        let payload = json!({"model": {"key": model_key}, "inputs": {"audio": input.payload()}});
        // model_id ties the job to a node that has this captioner (claim checks installations).
        sqlx::query(
            "INSERT INTO jobs (id, type, priority, owner_id, title, payload, model_id, max_attempts, created_at, updated_at) \
             SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, 2, ?8, ?8 \
             WHERE NOT EXISTS (SELECT 1 FROM jobs WHERE type = ?2 AND owner_id = ?4 \
               AND status IN ('queued', 'preparing', 'running') \
               AND json_extract(payload, '$.inputs.audio.artifact_id') = ?9)",
        )
        .bind(&job_id)
        .bind(JOB_TYPE)
        .bind(PRIORITY)
        .bind(&user.id)
        .bind(format!("Viết prompt: {}", input.filename))
        .bind(payload.to_string())
        .bind(model_id)
        .bind(&ts)
        .bind(&input.id)
        .execute(&state.db)
        .await?;
    }
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
    let (id, key) =
        live.ok_or_else(|| AppError::validation("Chưa có model mô tả trên máy render"))?;
    let status = if id == job_id {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(json!({"job_id": id, "model_key": key}))))
}

/// Check and tidy a worker's `describe_input` result before it is stored: a non-empty caption, cut to the
/// 2000-character prompt limit at a sentence end, the key in ACE-Step's keyscale form, a tempo in 20–300 BPM
/// (else null).
pub fn clean_result(result: &Value) -> AppResult<Value> {
    let text = |k: &str| result[k].as_str().unwrap_or_default().trim().to_string();
    let caption = cut_text(&text("caption"), MAX_CAPTION_CHARS);
    if caption.is_empty() {
        return Err(AppError::validation("result.caption không được rỗng"));
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
