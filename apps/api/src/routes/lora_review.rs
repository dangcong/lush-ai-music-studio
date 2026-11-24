//! LoRA Studio review: the jobs that follow a snapshot training run (`eval_lora`) and the
//! helpers that pin them to the node that holds the checkpoints.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{Sqlite, Transaction, types::Json as DbJson};

use crate::{
    AppState,
    auth::{AuthUser, Trainer},
    error::{AppError, AppResult},
    routes::{
        genres::{Preset, load_genre, validate_preset},
        jobs::{GenerationRequest, MAX_DURATION, MIN_DURATION, build_generation, insert_job},
        training::{CHECKPOINT_COLUMNS, Checkpoint},
    },
    util::{audit, new_id, now, now_plus},
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

/// `eval_00`..`eval_99`: the artifact kinds of listening renders.
pub fn is_eval_kind(kind: &str) -> bool {
    kind.strip_prefix("eval_")
        .is_some_and(|n| n.len() == 2 && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Adapter key of one saved epoch. A new key per epoch: the engine loads an adapter once by name.
pub fn checkpoint_key(base_key: &str, epoch: i64) -> String {
    format!("{base_key}-e{epoch}")
}

/// The render length: 60 s, pulled into the genre's own range when 60 lies outside it.
async fn render_duration(state: &AppState, genre_id: &str) -> AppResult<i64> {
    let genre = load_genre(state, genre_id).await?;
    Ok(match (genre.duration_min, genre.duration_max) {
        (Some(min), Some(max)) if min <= max => EVAL_DURATION.clamp(min, max),
        _ => EVAL_DURATION,
    }
    .clamp(MIN_DURATION, MAX_DURATION))
}

/// A plain text generate request on the catalog genre and base model, no LoRA: the shape shared by
/// listening renders and quick tries.
fn render_request(
    prompt: String,
    genre_id: &str,
    model_id: Option<String>,
    duration: i64,
    seed: Option<i64>,
) -> GenerationRequest {
    GenerationRequest {
        prompt,
        title: None,
        project_id: None,
        genre_id: Some(genre_id.to_string()).filter(|g| !g.is_empty()),
        subgenre_id: None,
        preset_id: None,
        model_id,
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
    }
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

    let genre_id = content["genre"]["id"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let model_id = content["base_model"]["id"].as_str().map(str::to_string);
    let duration = render_duration(state, &genre_id).await?;
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
        let req = render_request(prompt, &genre_id, model_id.clone(), duration, seed);
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

fn invalid_eval() -> AppError {
    AppError::validation("kết quả đánh giá không hợp lệ")
}

/// A score from the worker: a finite number, or null.
fn opt_number(value: &Value) -> AppResult<Option<f64>> {
    match value {
        Value::Null => Ok(None),
        v => v
            .as_f64()
            .filter(|n| n.is_finite())
            .map(Some)
            .ok_or_else(invalid_eval),
    }
}

/// A text value from the worker: a string, or null.
fn opt_text(value: &Value) -> AppResult<Option<String>> {
    match value {
        Value::Null => Ok(None),
        Value::String(s) => Ok(Some(s.chars().take(300).collect())),
        _ => Err(invalid_eval()),
    }
}

/// Record the result of an `eval_lora` job: the render rows and the per-checkpoint scores.
/// Every reference is checked first; the caller's transaction is dropped on error, so a bad
/// result writes nothing. `payload` is the job payload (`lora.id` names the LoRA).
pub(crate) async fn complete_eval(
    tx: &mut Transaction<'_, Sqlite>,
    job_id: &str,
    payload: &Value,
    result: &Value,
) -> AppResult<()> {
    let lora_id = payload["lora"]["id"].as_str().ok_or_else(invalid_eval)?;
    let renders = result["renders"].as_array().ok_or_else(invalid_eval)?;
    let none = vec![];
    let scored = match &result["checkpoints"] {
        Value::Null => &none,
        v => v.as_array().ok_or_else(invalid_eval)?,
    };

    // Validate everything, collect the rows, then write.
    let mut seen = std::collections::HashSet::new();
    let mut rows = vec![];
    for r in renders {
        let kind = r["artifact_kind"].as_str().filter(|k| is_eval_kind(k));
        let kind = kind.ok_or_else(invalid_eval)?;
        if !seen.insert(kind) {
            return Err(invalid_eval());
        }
        let artifact: Option<(String,)> =
            sqlx::query_as("SELECT id FROM artifacts WHERE job_id = ? AND kind = ?")
                .bind(job_id)
                .bind(kind)
                .fetch_optional(&mut **tx)
                .await?;
        let (artifact_id,) = artifact.ok_or_else(invalid_eval)?;
        let render_kind = r["kind"]
            .as_str()
            .filter(|k| matches!(*k, "base" | "prompt" | "held_out"))
            .ok_or_else(invalid_eval)?;
        let checkpoint_id = match &r["checkpoint_id"] {
            Value::Null => None,
            Value::String(id) => {
                owned_checkpoint(tx, lora_id, id).await?;
                Some(id.clone())
            }
            _ => return Err(invalid_eval()),
        };
        let prompt = r["prompt"].as_str().ok_or_else(invalid_eval)?;
        let scores = match &r["scores"] {
            Value::Null => json!({}),
            v if v.is_object() => v.clone(),
            _ => return Err(invalid_eval()),
        };
        rows.push((
            artifact_id,
            render_kind,
            checkpoint_id,
            opt_text(&r["sample_filename"])?,
            prompt,
            scores,
        ));
    }
    let mut updates = vec![];
    for c in scored {
        let id = c["id"].as_str().ok_or_else(invalid_eval)?;
        owned_checkpoint(tx, lora_id, id).await?;
        updates.push((
            id,
            opt_number(&c["style"])?,
            opt_number(&c["train"])?,
            opt_text(&c["train_match"])?,
            opt_number(&c["melody"])?,
            opt_text(&c["melody_match"])?,
            opt_number(&c["quality"])?,
        ));
    }

    let ts = now();
    for (artifact_id, kind, checkpoint_id, sample, prompt, scores) in rows {
        sqlx::query(
            "INSERT INTO lora_eval_renders (id, lora_id, job_id, checkpoint_id, kind, sample_filename, \
             prompt, artifact_id, scores, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(new_id())
        .bind(lora_id)
        .bind(job_id)
        .bind(checkpoint_id)
        .bind(kind)
        .bind(sample)
        .bind(prompt)
        .bind(artifact_id)
        .bind(scores.to_string())
        .bind(&ts)
        .execute(&mut **tx)
        .await?;
    }
    for (id, style, train, train_match, melody, melody_match, quality) in updates {
        sqlx::query(
            "UPDATE lora_checkpoints SET style_score = ?, train_score = ?, train_match = ?, \
             melody_score = ?, melody_match = ?, quality_score = ? WHERE id = ? AND lora_id = ?",
        )
        .bind(style)
        .bind(train)
        .bind(train_match)
        .bind(melody)
        .bind(melody_match)
        .bind(quality)
        .bind(id)
        .bind(lora_id)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// The checkpoint must belong to the LoRA of the job.
async fn owned_checkpoint(
    tx: &mut Transaction<'_, Sqlite>,
    lora_id: &str,
    checkpoint_id: &str,
) -> AppResult<()> {
    let found: Option<(i64,)> =
        sqlx::query_as("SELECT 1 FROM lora_checkpoints WHERE id = ? AND lora_id = ?")
            .bind(checkpoint_id)
            .bind(lora_id)
            .fetch_optional(&mut **tx)
            .await?;
    found.map(|_| ()).ok_or_else(invalid_eval)
}

// ---------- files jobs: publish a checkpoint, delete checkpoints ----------

pub const JOB_FILES: &str = "lora_files";
/// A checkpoint nobody chose is deleted this long after the choice.
const KEEP_DAYS: i64 = 30;
/// A file job of the LoRA is already waiting or running.
const FILES_BUSY: &str = "LoRA đang được cập nhật trên máy, chờ một chút";
/// A delete mark with no finished job after this long is retried by the heartbeat.
const ABANDONED_AFTER_SECS: i64 = 86_400;
const MAX_NOTE_CHARS: usize = 500;
const MAX_STYLE_NAME_CHARS: usize = 80;

/// Where the LoRA was trained and who made it: (node id, owner id). Both live on the train job.
async fn train_origin(
    tx: &mut Transaction<'_, Sqlite>,
    lora_id: &str,
) -> AppResult<(String, String)> {
    let row: Option<(Option<String>, String)> = sqlx::query_as(
        "SELECT j.node_id, j.owner_id FROM loras l JOIN jobs j ON j.id = l.job_id WHERE l.id = ?",
    )
    .bind(lora_id)
    .fetch_optional(&mut **tx)
    .await?;
    match row {
        Some((Some(node), owner)) => Ok((node, owner)),
        _ => Err(AppError::conflict(
            "Không tìm thấy máy đang giữ các bản lưu của LoRA này",
        )),
    }
}

/// Queue a `lora_files` job pinned to `node_id`. `payload` gets `node_id`, `lora_id` and `lora.id`.
async fn insert_files_job(
    tx: &mut Transaction<'_, Sqlite>,
    owner_id: &str,
    node_id: &str,
    lora_id: &str,
    title: String,
    mut payload: Value,
) -> AppResult<String> {
    payload["node_id"] = json!(node_id);
    payload["lora_id"] = json!(lora_id);
    payload["lora"] = json!({"id": lora_id});
    let (job_id, ts) = (new_id(), now());
    sqlx::query(
        "INSERT INTO jobs (id, type, priority, owner_id, title, payload, max_attempts, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, 2, ?, ?)",
    )
    .bind(&job_id)
    .bind(JOB_FILES)
    .bind(FILES_PRIORITY)
    .bind(owner_id)
    .bind(title)
    .bind(payload.to_string())
    .bind(&ts)
    .bind(&ts)
    .execute(&mut **tx)
    .await
    // `jobs_one_live_lora_files`: a second live file job for the same LoRA.
    .map_err(|e| match AppError::from(e) {
        e if e.status == StatusCode::CONFLICT => AppError::conflict(FILES_BUSY),
        e => e,
    })?;
    Ok(job_id)
}

/// Queue one `delete` job for the checkpoints of `lora_id` that `status` and `filter` select
/// (`filter` is SQL appended after `status`, with `bind` values). The caller guarantees no
/// `lora_files` job of the LoRA is live, so a checkpoint that is already marked
/// (`delete_requested_at`) was abandoned by a failed job and is picked again. Every selected
/// checkpoint is (re)marked. `None` when there is nothing to delete.
async fn queue_delete(
    tx: &mut Transaction<'_, Sqlite>,
    lora_id: &str,
    only_kept: bool,
    extra_where: &str,
    bind: &[&str],
    title: &str,
) -> AppResult<Option<(String, usize)>> {
    let status = if only_kept {
        "status = 'kept'"
    } else {
        "status != 'deleted'"
    };
    let sql = format!(
        "SELECT id, node_path FROM lora_checkpoints WHERE lora_id = ? AND {status} \
         {extra_where} ORDER BY epoch"
    );
    let mut q = sqlx::query_as::<_, (String, String)>(&sql).bind(lora_id);
    for b in bind {
        q = q.bind(*b);
    }
    let rows = q.fetch_all(&mut **tx).await?;
    if rows.is_empty() {
        return Ok(None);
    }
    let (node_id, owner_id) = train_origin(tx, lora_id).await?;
    let ts = now();
    for (id, _) in &rows {
        sqlx::query("UPDATE lora_checkpoints SET delete_requested_at = ? WHERE id = ?")
            .bind(&ts)
            .bind(id)
            .execute(&mut **tx)
            .await?;
    }
    let checkpoints: Vec<Value> = rows
        .iter()
        .map(|(id, path)| json!({"id": id, "path": path}))
        .collect();
    let job_id = insert_files_job(
        tx,
        &owner_id,
        &node_id,
        lora_id,
        title.to_string(),
        json!({"op": "delete", "checkpoints": checkpoints}),
    )
    .await?;
    Ok(Some((job_id, rows.len())))
}

async fn live_files_job(tx: &mut Transaction<'_, Sqlite>, lora_id: &str) -> AppResult<bool> {
    let live: Option<(i64,)> = sqlx::query_as(
        "SELECT 1 FROM jobs WHERE type = 'lora_files' AND status IN ('queued', 'preparing', 'running') \
         AND json_extract(payload, '$.lora_id') = ? LIMIT 1",
    )
    .bind(lora_id)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(live.is_some())
}

// ---------- routes ----------

#[derive(Deserialize)]
pub struct NoteBody {
    note: String,
}

/// `PATCH /lora-checkpoints/{id}`: the listener's note on one epoch.
pub async fn update_checkpoint_note(
    State(state): State<AppState>,
    Trainer(_user): Trainer,
    Path(id): Path<String>,
    Json(body): Json<NoteBody>,
) -> AppResult<Json<Checkpoint>> {
    if body.note.chars().count() > MAX_NOTE_CHARS {
        return Err(AppError::validation("Ghi chú tối đa 500 ký tự"));
    }
    let done = sqlx::query("UPDATE lora_checkpoints SET note = ? WHERE id = ?")
        .bind(&body.note)
        .bind(&id)
        .execute(&state.db)
        .await?;
    if done.rows_affected() == 0 {
        return Err(AppError::not_found("lora checkpoint"));
    }
    let checkpoint = sqlx::query_as(&format!(
        "SELECT {CHECKPOINT_COLUMNS} FROM lora_checkpoints c WHERE c.id = ?"
    ))
    .bind(&id)
    .fetch_one(&state.db)
    .await?;
    Ok(Json(checkpoint))
}

#[derive(Deserialize)]
pub struct PublishBody {
    checkpoint_id: String,
    style_name: String,
    lora_scale: f64,
}

/// `POST /loras/{id}/publish`: make one epoch the LoRA's chosen one and put it in its Style.
pub async fn publish(
    State(state): State<AppState>,
    Trainer(user): Trainer,
    Path(lora_id): Path<String>,
    Json(body): Json<PublishBody>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let mut tx = state.db.begin().await?;
    #[allow(clippy::type_complexity)]
    let lora: Option<(
        String,
        Option<String>,
        String,
        Option<String>,
        String,
        String,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT l.key, l.base_key, l.name, l.genre_id, l.base_model_id, l.status, s.content \
             FROM loras l LEFT JOIN dataset_snapshots s ON s.id = l.snapshot_id WHERE l.id = ?",
    )
    .bind(&lora_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (key, base_key, _name, genre_id, model_id, status, snapshot) =
        lora.ok_or_else(|| AppError::not_found("lora"))?;
    if status == "archived" {
        return Err(AppError::validation("LoRA đã bị xóa"));
    }
    if live_files_job(&mut tx, &lora_id).await? {
        return Err(AppError::conflict(
            "LoRA đang được cập nhật trên máy, chờ một chút",
        ));
    }
    let cp: Option<(i64, String, String, String)> = sqlx::query_as(
        "SELECT epoch, node_path, sha256, status FROM lora_checkpoints \
         WHERE id = ? AND lora_id = ? AND status != 'deleted' AND delete_requested_at IS NULL",
    )
    .bind(&body.checkpoint_id)
    .bind(&lora_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (epoch, node_path, sha256, cp_status) =
        cp.ok_or_else(|| AppError::validation("Bản lưu này không còn trên máy"))?;
    let style_name = body.style_name.trim().to_string();
    if style_name.is_empty() || style_name.chars().count() > MAX_STYLE_NAME_CHARS {
        return Err(AppError::validation("Tên Style phải có 1–80 ký tự"));
    }
    if !(0.0..=1.0).contains(&body.lora_scale) {
        return Err(AppError::validation("Độ mạnh phải trong khoảng 0–1"));
    }

    let snapshot: Value = snapshot
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null);
    let genre_id = genre_id
        .or_else(|| snapshot["genre"]["id"].as_str().map(str::to_string))
        .ok_or_else(|| AppError::validation("LoRA chưa có genre để tạo Style"))?;
    let keyword = snapshot["keyword"].as_str().unwrap_or_default().to_string();

    // The chosen epoch is already on the node: only the Style changes.
    let installed: Option<(i64,)> =
        sqlx::query_as("SELECT 1 FROM lora_installations WHERE lora_id = ? LIMIT 1")
            .bind(&lora_id)
            .fetch_optional(&mut *tx)
            .await?;
    let already = cp_status == "chosen" && installed.is_some();
    let base = base_key.unwrap_or_else(|| key.clone());
    let target_key = checkpoint_key(&base, epoch);

    let existing: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM style_presets WHERE kind = 'trained' AND lora_id = ? ORDER BY id LIMIT 1",
    )
    .bind(&lora_id)
    .fetch_optional(&mut *tx)
    .await?;
    let style_id = match existing {
        Some((id,)) => {
            sqlx::query("UPDATE style_presets SET name = ?, lora_scale = ? WHERE id = ?")
                .bind(&style_name)
                .bind(body.lora_scale)
                .bind(&id)
                .execute(&mut *tx)
                .await?;
            id
        }
        None => {
            let preset = Preset {
                id: new_id(),
                genre_id,
                name: style_name.clone(),
                description: String::new(),
                kind: "trained".into(),
                prompt_template: keyword,
                instruments: DbJson(vec![]),
                moods: DbJson(vec![]),
                bpm: None,
                duration_min: None,
                duration_max: None,
                model_id: Some(model_id),
                lora_id: Some(lora_id.clone()),
                lora_scale: Some(body.lora_scale),
                params: DbJson(json!({})),
                version: 1,
                is_active: already,
            };
            validate_preset(&preset)?;
            sqlx::query(
                "INSERT INTO style_presets (id, genre_id, name, description, kind, prompt_template, \
                 instruments, moods, model_id, lora_id, lora_scale, params, version, is_active) \
                 VALUES (?, ?, ?, '', 'trained', ?, '[]', '[]', ?, ?, ?, '{}', 1, ?)",
            )
            .bind(&preset.id)
            .bind(&preset.genre_id)
            .bind(&preset.name)
            .bind(&preset.prompt_template)
            .bind(&preset.model_id)
            .bind(&preset.lora_id)
            .bind(preset.lora_scale)
            .bind(preset.is_active)
            .execute(&mut *tx)
            .await?;
            preset.id
        }
    };

    if already {
        tx.commit().await?;
        audit(
            &state.db,
            Some(&user.id),
            "lora.published",
            "lora",
            &lora_id,
            json!({"checkpoint_id": body.checkpoint_id, "epoch": epoch, "style_preset_id": style_id}),
        )
        .await;
        return Ok((
            StatusCode::OK,
            Json(json!({"job_id": null, "style_preset_id": style_id})),
        ));
    }
    let (node_id, owner_id) = train_origin(&mut tx, &lora_id).await?;
    let job_id = insert_files_job(
        &mut tx,
        &owner_id,
        &node_id,
        &lora_id,
        format!("Đưa {style_name} vào Style"),
        json!({"op": "publish", "target_key": target_key,
               "checkpoint": {"id": body.checkpoint_id, "path": node_path, "sha256": sha256}}),
    )
    .await?;
    tx.commit().await?;
    audit(
        &state.db,
        Some(&user.id),
        "lora.published",
        "lora",
        &lora_id,
        json!({"checkpoint_id": body.checkpoint_id, "epoch": epoch, "style_preset_id": style_id}),
    )
    .await;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({"job_id": job_id, "style_preset_id": style_id})),
    ))
}

/// `POST /loras/{id}/cleanup`: delete the epochs nobody chose, now instead of at their deadline.
pub async fn cleanup(
    State(state): State<AppState>,
    Trainer(user): Trainer,
    Path(lora_id): Path<String>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let mut tx = state.db.begin().await?;
    let row: Option<(String,)> = sqlx::query_as("SELECT status FROM loras WHERE id = ?")
        .bind(&lora_id)
        .fetch_optional(&mut *tx)
        .await?;
    let (status,) = row.ok_or_else(|| AppError::not_found("lora"))?;
    if status == "archived" {
        return Err(AppError::validation("LoRA đã bị xóa"));
    }
    let chosen: Option<(i64,)> = sqlx::query_as(
        "SELECT 1 FROM lora_checkpoints WHERE lora_id = ? AND status = 'chosen' LIMIT 1",
    )
    .bind(&lora_id)
    .fetch_optional(&mut *tx)
    .await?;
    if chosen.is_none() {
        return Err(AppError::conflict(
            "Chọn một bản trước khi xóa các bản còn lại",
        ));
    }
    let listening: Option<(i64,)> = sqlx::query_as(
        "SELECT 1 FROM jobs WHERE type = 'eval_lora' AND status IN ('queued', 'preparing', 'running') \
         AND json_extract(payload, '$.lora.id') = ? LIMIT 1",
    )
    .bind(&lora_id)
    .fetch_optional(&mut *tx)
    .await?;
    if listening.is_some() {
        return Err(AppError::conflict(
            "Đang tạo nhạc nghe thử, chờ xong rồi xóa",
        ));
    }
    if live_files_job(&mut tx, &lora_id).await? {
        return Err(AppError::conflict(FILES_BUSY));
    }
    let queued = queue_delete(
        &mut tx,
        &lora_id,
        true,
        "",
        &[],
        "Xóa các bản lưu không chọn",
    )
    .await?;
    tx.commit().await?;
    let Some((job_id, count)) = queued else {
        return Ok((StatusCode::OK, Json(json!({"job_id": null, "count": 0}))));
    };
    audit(
        &state.db,
        Some(&user.id),
        "lora.cleanup",
        "lora",
        &lora_id,
        json!({"count": count}),
    )
    .await;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({"job_id": job_id, "count": count})),
    ))
}

/// `DELETE /loras/{id}`: archive the LoRA and delete every checkpoint it still has on the node.
pub async fn delete_lora(
    State(state): State<AppState>,
    Trainer(user): Trainer,
    Path(lora_id): Path<String>,
) -> AppResult<StatusCode> {
    let mut tx = state.db.begin().await?;
    let row: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT status, job_id FROM loras WHERE id = ?")
            .bind(&lora_id)
            .fetch_optional(&mut *tx)
            .await?;
    let (status, train_job) = row.ok_or_else(|| AppError::not_found("lora"))?;
    if status == "archived" {
        return Err(AppError::validation("LoRA đã bị xóa"));
    }
    let styles: Vec<(String,)> =
        sqlx::query_as("SELECT name FROM style_presets WHERE lora_id = ? ORDER BY name")
            .bind(&lora_id)
            .fetch_all(&mut *tx)
            .await?;
    if !styles.is_empty() {
        let names: Vec<String> = styles.into_iter().map(|(n,)| n).collect();
        return Err(AppError::conflict(format!(
            "Còn Style dùng LoRA này: {}",
            names.join(", ")
        )));
    }
    // Work on the node: training or listening that runs now, or a file operation not finished.
    let busy: Option<(i64,)> = sqlx::query_as(
        "SELECT 1 FROM jobs WHERE (id = ?1 AND status IN ('preparing', 'running')) \
            OR (type = 'eval_lora' AND status IN ('preparing', 'running') AND json_extract(payload, '$.lora.id') = ?2) \
            OR (type = 'lora_files' AND status IN ('queued', 'preparing', 'running') AND json_extract(payload, '$.lora_id') = ?2) \
         LIMIT 1",
    )
    .bind(train_job.unwrap_or_default())
    .bind(&lora_id)
    .fetch_optional(&mut *tx)
    .await?;
    if busy.is_some() {
        return Err(AppError::conflict("LoRA đang có việc chạy trên máy"));
    }
    let ts = now();
    sqlx::query("UPDATE loras SET status = 'archived' WHERE id = ?")
        .bind(&lora_id)
        .execute(&mut *tx)
        .await?;
    // Listening that was only queued has nothing left to listen to.
    sqlx::query(
        "UPDATE jobs SET status = 'cancelled', finished_at = ?1, updated_at = ?1 \
         WHERE type = 'eval_lora' AND status = 'queued' AND json_extract(payload, '$.lora.id') = ?2",
    )
    .bind(&ts)
    .bind(&lora_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM lora_installations WHERE lora_id = ?")
        .bind(&lora_id)
        .execute(&mut *tx)
        .await?;
    let queued = queue_delete(&mut tx, &lora_id, false, "", &[], "Xóa LoRA").await?;
    tx.commit().await?;
    audit(
        &state.db,
        Some(&user.id),
        "lora.deleted",
        "lora",
        &lora_id,
        json!({"checkpoints": queued.map_or(0, |(_, n)| n)}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// Heartbeat hook: when the node has no file job waiting, queue one `delete` for the first LoRA
/// that has checkpoints past their deadline. The chosen checkpoint has no deadline.
pub(crate) async fn queue_expired_cleanup(state: &AppState, node_id: &str) -> AppResult<()> {
    let mut tx = state.db.begin().await?;
    let waiting: Option<(i64,)> = sqlx::query_as(
        "SELECT 1 FROM jobs WHERE type = 'lora_files' AND status IN ('queued', 'preparing', 'running') \
         AND json_extract(payload, '$.node_id') = ? LIMIT 1",
    )
    .bind(node_id)
    .fetch_optional(&mut *tx)
    .await?;
    if waiting.is_some() {
        return Ok(());
    }
    // Due: a kept epoch past its deadline, or a delete mark that no job finished for a day
    // (also on an archived LoRA, whose delete job failed).
    let (ts, stale) = (now(), now_plus(-ABANDONED_AFTER_SECS));
    let due: Option<(String,)> = sqlx::query_as(
        "SELECT c.lora_id FROM lora_checkpoints c JOIN loras l ON l.id = c.lora_id \
         JOIN jobs t ON t.id = l.job_id \
         WHERE t.node_id = ?1 AND c.status != 'deleted' AND ( \
               (c.status = 'kept' AND c.expires_at < ?2 AND c.delete_requested_at IS NULL \
                AND l.status != 'archived') \
            OR c.delete_requested_at < ?3) \
         ORDER BY COALESCE(c.expires_at, c.delete_requested_at) LIMIT 1",
    )
    .bind(node_id)
    .bind(&ts)
    .bind(&stale)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((lora_id,)) = due else {
        return Ok(());
    };
    let queued = queue_delete(
        &mut tx,
        &lora_id,
        false,
        "AND ((status = 'kept' AND expires_at < ? AND delete_requested_at IS NULL) \
         OR delete_requested_at < ?)",
        &[&ts, &stale],
        "Xóa các bản lưu hết hạn",
    )
    .await;
    let queued = match queued {
        Ok(q) => q,
        // A file job of this LoRA is already live (the unique index): nothing to queue.
        Err(e) if e.status == StatusCode::CONFLICT => return Ok(()),
        Err(e) => return Err(e),
    };
    tx.commit().await?;
    if let Some((job_id, count)) = queued {
        tracing::info!(node = %node_id, lora = %lora_id, job = %job_id, count, "expired checkpoints queued for deletion");
    }
    Ok(())
}

fn invalid_files() -> AppError {
    AppError::validation("kết quả xử lý file không hợp lệ")
}

/// Ids named by `result[field]`: an array of strings, each one of the job's `allowed` ids.
fn listed_ids(result: &Value, field: &str, allowed: &[&str]) -> AppResult<Vec<String>> {
    let list = match &result[field] {
        Value::Null => return Ok(vec![]),
        Value::Array(a) => a,
        _ => return Err(invalid_files()),
    };
    list.iter()
        .map(|v| {
            v.as_str()
                .filter(|id| allowed.contains(id))
                .map(str::to_string)
                .ok_or_else(invalid_files)
        })
        .collect()
}

/// Record the result of a `lora_files` job. A bad result writes nothing (the caller drops its
/// transaction on error).
pub(crate) async fn complete_files(
    tx: &mut Transaction<'_, Sqlite>,
    node_id: &str,
    payload: &Value,
    result: &Value,
) -> AppResult<()> {
    let lora_id = payload["lora_id"].as_str().ok_or_else(invalid_files)?;
    match payload["op"].as_str() {
        Some("publish") => {
            let published = &result["published"];
            let published_key = payload["target_key"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            if published["key"].as_str() != payload["target_key"].as_str()
                || payload["target_key"].is_null()
                || published["sha256"].as_str() != payload["checkpoint"]["sha256"].as_str()
                || published["sha256"].is_null()
            {
                return Err(AppError::validation(
                    "Node báo bản đã chép không khớp với bản được chọn",
                ));
            }
            // The copy is on the node: only now does the choice take effect.
            let checkpoint_id = payload["checkpoint"]["id"]
                .as_str()
                .ok_or_else(invalid_files)?;
            let deadline = now_plus(KEEP_DAYS * 86_400);
            let owned = sqlx::query(
                "UPDATE lora_checkpoints SET status = 'chosen', expires_at = NULL                  WHERE id = ? AND lora_id = ? AND status != 'deleted'",
            )
            .bind(checkpoint_id)
            .bind(lora_id)
            .execute(&mut **tx)
            .await?;
            if owned.rows_affected() == 0 {
                return Err(invalid_files());
            }
            // Every other epoch still on the node waits for its deadline (an earlier one is kept).
            sqlx::query(
                "UPDATE lora_checkpoints SET status = 'kept', expires_at = COALESCE(expires_at, ?)                  WHERE lora_id = ? AND id != ? AND status IN ('chosen', 'kept')",
            )
            .bind(&deadline)
            .bind(lora_id)
            .bind(checkpoint_id)
            .execute(&mut **tx)
            .await?;
            sqlx::query(
                "UPDATE loras SET base_key = COALESCE(base_key, key), key = ?,                  chosen_checkpoint_id = ?, status = 'approved' WHERE id = ? AND status != 'archived'",
            )
            .bind(&published_key)
            .bind(checkpoint_id)
            .bind(lora_id)
            .execute(&mut **tx)
            .await?;
            sqlx::query("DELETE FROM lora_installations WHERE lora_id = ?")
                .bind(lora_id)
                .execute(&mut **tx)
                .await?;
            sqlx::query("INSERT INTO lora_installations (node_id, lora_id) VALUES (?, ?)")
                .bind(node_id)
                .bind(lora_id)
                .execute(&mut **tx)
                .await?;
            sqlx::query(
                "UPDATE style_presets SET is_active = 1 WHERE kind = 'trained' AND lora_id = ?",
            )
            .bind(lora_id)
            .execute(&mut **tx)
            .await?;
        }
        Some("delete") => {
            let asked: Vec<&str> = payload["checkpoints"]
                .as_array()
                .map(|a| a.iter().filter_map(|c| c["id"].as_str()).collect())
                .unwrap_or_default();
            let mut ids = listed_ids(result, "deleted", &asked)?;
            ids.extend(listed_ids(result, "missing", &asked)?);
            for id in ids {
                sqlx::query(
                    "UPDATE lora_checkpoints SET status = 'deleted', expires_at = NULL WHERE id = ? AND lora_id = ?",
                )
                .bind(id)
                .bind(lora_id)
                .execute(&mut **tx)
                .await?;
            }
        }
        _ => return Err(invalid_files()),
    }
    Ok(())
}

// ---------- quick try ----------

#[derive(Deserialize)]
pub struct TryBody {
    checkpoint_id: String,
    prompt: String,
    lora_scale: f64,
}

/// `POST /loras/{id}/try`: render one saved epoch as a `generate` job of the caller, pinned to the
/// node that holds the file. The job carries the file path in its payload only (never in `request`),
/// and has no `lora_id`: the epoch is not installed in the Registry sense.
#[allow(clippy::type_complexity)]
pub async fn quick_try(
    State(state): State<AppState>,
    Trainer(user): Trainer,
    Path(lora_id): Path<String>,
    Json(body): Json<TryBody>,
) -> AppResult<(StatusCode, Json<crate::queue::Job>)> {
    let row: Option<(String, Option<String>, String, String, Option<String>, Option<String>)> =
        sqlx::query_as(
            "SELECT l.key, l.base_key, l.name, l.base_model_id, l.genre_id, s.content              FROM loras l LEFT JOIN dataset_snapshots s ON s.id = l.snapshot_id WHERE l.id = ?",
        )
        .bind(&lora_id)
        .fetch_optional(&state.db)
        .await?;
    let (key, base_key, name, model_id, genre_id, snapshot) =
        row.ok_or_else(|| AppError::not_found("lora"))?;
    let status: String = sqlx::query_scalar("SELECT status FROM loras WHERE id = ?")
        .bind(&lora_id)
        .fetch_one(&state.db)
        .await?;
    if status == "archived" {
        return Err(AppError::validation("LoRA đã bị xóa"));
    }
    let prompt = body.prompt.trim().to_string();
    if prompt.is_empty() || prompt.chars().count() > 500 {
        return Err(AppError::validation("Câu lệnh phải có 1–500 ký tự"));
    }
    let cp: Option<(i64, String)> = sqlx::query_as(
        "SELECT epoch, node_path FROM lora_checkpoints          WHERE id = ? AND lora_id = ? AND status != 'deleted' AND delete_requested_at IS NULL",
    )
    .bind(&body.checkpoint_id)
    .bind(&lora_id)
    .fetch_optional(&state.db)
    .await?;
    let (epoch, node_path) =
        cp.ok_or_else(|| AppError::validation("Bản lưu này không còn trên máy"))?;
    if !(0.0..=1.0).contains(&body.lora_scale) {
        return Err(AppError::validation("Độ mạnh phải trong khoảng 0–1"));
    }
    let snapshot: Value = snapshot
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null);
    let genre_id = genre_id
        .or_else(|| snapshot["genre"]["id"].as_str().map(str::to_string))
        .ok_or_else(|| AppError::validation("LoRA chưa có genre để thử"))?;

    let mut tx = state.db.begin().await?;
    let (node_id, _) = train_origin(&mut tx, &lora_id).await?;
    drop(tx);

    let duration = render_duration(&state, &genre_id).await?;
    let mut req = render_request(prompt, &genre_id, Some(model_id), duration, None);
    req.title = Some(format!("Thử nhanh · {name} · Epoch {epoch}"));
    let mut built = build_generation(&state, &user, &req).await?;
    // The adapter key follows the epoch, never the current `loras.key` (which may already end in -eN).
    let adapter = checkpoint_key(base_key.as_deref().unwrap_or(&key), epoch);
    built.payload["lora"] = json!({"key": adapter, "scale": body.lora_scale, "path": node_path});
    built.payload["node_id"] = json!(node_id);
    let id = insert_job(
        &state,
        &user.id,
        None,
        &built.title,
        &built.request,
        &built.payload,
        &built.model_id,
        None,
        None,
    )
    .await?;
    audit(
        &state.db,
        Some(&user.id),
        "lora.tried",
        "lora",
        &lora_id,
        json!({"epoch": epoch, "job_id": id}),
    )
    .await;
    let job = crate::queue::load_job(&state.db, &id)
        .await?
        .ok_or_else(|| AppError::not_found("job"))?;
    Ok((StatusCode::CREATED, Json(job)))
}
