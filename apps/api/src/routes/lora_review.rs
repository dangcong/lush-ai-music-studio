//! LoRA Studio review: the jobs that follow a snapshot training run (`eval_lora`) and the
//! helpers that pin them to the node that holds the checkpoints.

use serde_json::{Value, json};
use sqlx::{Sqlite, Transaction};

use crate::{
    AppState,
    auth::AuthUser,
    error::{AppError, AppResult},
    routes::{
        genres::load_genre,
        jobs::{GenerationRequest, MAX_DURATION, MIN_DURATION, build_generation},
    },
    util::{new_id, now},
};

pub const JOB_EVAL: &str = "eval_lora";
/// `generate` is 0, `analyze_track` -10, `caption_tracks` -20.
pub const EVAL_PRIORITY: i64 = -30;
/// Training runs created from a snapshot wait behind everything else.
pub const TRAIN_PRIORITY: i64 = -40;
/// File jobs on a node (publish, delete) take a few seconds and use no GPU.
pub const FILES_PRIORITY: i64 = 10;

/// Listening renders use the epoch checkpoints, at most this many (the last ones).
pub const MAX_EVAL_CHECKPOINTS: i64 = 10;
/// Render length and LoRA strength of the listening renders.
const EVAL_DURATION: i64 = 60;
const EVAL_SCALE: f64 = 0.7;

/// Longest prompt a generate request accepts (`build_generation`).
const MAX_PROMPT_CHARS: usize = 2000;

/// `text` cut to at most `max` characters, at the last whitespace before the limit (no ellipsis).
fn cut_at_whitespace(text: &str, max: usize) -> &str {
    let text = text.trim();
    if text.chars().count() <= max {
        return text;
    }
    let end = text.char_indices().nth(max).map_or(text.len(), |(i, _)| i);
    let head = &text[..end];
    // Cut mid-word only when the head holds no whitespace at all.
    match head.rfind(char::is_whitespace) {
        Some(i) if i > 0 => head[..i].trim_end(),
        _ => head,
    }
}

/// Adapter key of one saved epoch. A new key per epoch: the engine loads an adapter once by name.
pub fn checkpoint_key(base_key: &str, epoch: i64) -> String {
    format!("{base_key}-e{epoch}")
}

/// Queue the `eval_lora` job for a finished snapshot run, inside the completion transaction.
/// Returns the new job id. The job is pinned to `node_id`, where the checkpoints live.
pub(crate) async fn enqueue_eval(
    state: &AppState,
    tx: &mut Transaction<'_, Sqlite>,
    lora_id: &str,
    train_job_id: &str,
    node_id: &str,
) -> AppResult<String> {
    let row: Option<(String, String, String, String)> = sqlx::query_as(
        "SELECT s.content, s.created_by, l.key, l.name FROM loras l \
         JOIN dataset_snapshots s ON s.id = l.snapshot_id WHERE l.id = ? AND l.job_id = ?",
    )
    .bind(lora_id)
    .bind(train_job_id)
    .fetch_optional(&mut **tx)
    .await?;
    let (content, created_by, lora_key, lora_name) =
        row.ok_or_else(|| AppError::not_found("dataset snapshot"))?;
    let content: Value = serde_json::from_str(&content)?;

    let rows: Vec<(String, i64, String, String)> = sqlx::query_as(
        "SELECT id, epoch, node_path, sha256 FROM lora_checkpoints \
         WHERE lora_id = ? AND status != 'deleted' ORDER BY epoch DESC LIMIT ?",
    )
    .bind(lora_id)
    .bind(MAX_EVAL_CHECKPOINTS)
    .fetch_all(&mut **tx)
    .await?;
    let checkpoints: Vec<Value> = rows
        .into_iter()
        .rev()
        .map(|(id, epoch, path, sha)| {
            json!({"id": id, "epoch": epoch, "key": checkpoint_key(&lora_key, epoch),
                   "path": path, "sha256": sha})
        })
        .collect();

    // The render length: 60 s, pulled into the genre's own range when 60 lies outside it.
    let genre = load_genre(state, content["genre"]["id"].as_str().unwrap_or_default()).await?;
    let duration = match (genre.duration_min, genre.duration_max) {
        (Some(min), Some(max)) if min <= max => EVAL_DURATION.clamp(min, max),
        _ => EVAL_DURATION,
    }
    .clamp(MIN_DURATION, MAX_DURATION);
    let seed = content["config"]["seed"].as_i64();
    let test_prompt = content["test_prompt"].as_str().unwrap_or_default();

    let (username, role): (String, String) =
        sqlx::query_as("SELECT username, role FROM users WHERE id = ?")
            .bind(&created_by)
            .fetch_one(&mut **tx)
            .await?;
    let owner = AuthUser {
        id: created_by.clone(),
        username,
        role,
        session_hash: String::new(),
    };
    // Every render prompt goes through the same composition (genre template, tags), so the worker
    // can use them verbatim. Only the prompt differs between requests.
    let build = |prompt: String| async {
        let req = GenerationRequest {
            prompt,
            title: None,
            project_id: None,
            genre_id: content["genre"]["id"].as_str().map(str::to_string),
            subgenre_id: None,
            preset_id: None,
            model_id: content["base_model"]["id"].as_str().map(str::to_string),
            lora_id: None,
            lora_scale: None,
            duration,
            bpm: None,
            key: None,
            time_signature: None,
            seed,
            instruments: None,
            moods: None,
            instrumental: true,
            params: None,
            mode: None,
            reference_audio_id: None,
            source_audio_id: None,
            idempotency_key: None,
        };
        build_generation(state, &owner, &req).await
    };
    let built = build(test_prompt.to_string()).await?;
    let composed_test_prompt = built.payload["prompt"].clone();

    let samples = content["samples"].as_array().cloned().unwrap_or_default();
    let train: Vec<Value> = samples
        .iter()
        .enumerate()
        .map(|(i, s)| json!({"dataset_file": format!("s{i:03}.wav"), "filename": s["filename"]}))
        .collect();
    let mut held_out: Vec<Value> = vec![];
    for sample in content["held_out"].as_array().cloned().unwrap_or_default() {
        let caption = sample["train_caption"].as_str().unwrap_or_default();
        // A long caption must not block the listening job: cut it until the composed prompt fits.
        let mut limit = MAX_PROMPT_CHARS;
        let composed = loop {
            let built = build(cut_at_whitespace(caption, limit).to_string()).await?;
            let composed = built.payload["prompt"].clone();
            let len = composed.as_str().map_or(0, |p| p.chars().count());
            if len <= MAX_PROMPT_CHARS || limit <= 1 {
                break composed;
            }
            limit = limit.saturating_sub(len - MAX_PROMPT_CHARS).max(1);
        };
        held_out.push(json!({"filename": sample["filename"], "prompt": composed}));
    }

    let payload = json!({
        "node_id": node_id,
        "lora": {"id": lora_id, "base_key": lora_key},
        "dataset_key": lora_key,
        "scale": EVAL_SCALE, "duration": duration, "seed": seed,
        "render_template": built.payload,
        "test_prompt": composed_test_prompt,
        "checkpoints": checkpoints, "train": train, "held_out": held_out,
    });
    let (job_id, ts) = (new_id(), now());
    sqlx::query(
        "INSERT INTO jobs (id, type, priority, owner_id, title, payload, max_attempts, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, 2, ?, ?)",
    )
    .bind(&job_id)
    .bind(JOB_EVAL)
    .bind(EVAL_PRIORITY)
    .bind(&created_by)
    .bind(format!("Nghe thử {lora_name}"))
    .bind(payload.to_string())
    .bind(&ts)
    .bind(&ts)
    .execute(&mut **tx)
    .await?;
    Ok(job_id)
}
