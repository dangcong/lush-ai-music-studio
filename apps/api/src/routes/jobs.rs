//! Music generation requests and the user-facing side of the job queue.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sqlx::{QueryBuilder, Sqlite, types::Json as DbJson};

use crate::{
    AppState,
    auth::AuthUser,
    error::{AppError, AppResult},
    queue::{JOB_SELECT, Job, load_job},
    routes::{
        genres::{load_genre, load_preset},
        inputs::load_audio_input,
        registry::online_cutoff,
        training,
    },
    util::{new_id, now, random_seed},
};

pub const MIN_DURATION: i64 = 10;
pub const MAX_DURATION: i64 = 300;

#[derive(Serialize, Deserialize)]
pub struct GenerationRequest {
    pub prompt: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub genre_id: Option<String>,
    #[serde(default)]
    pub subgenre_id: Option<String>,
    #[serde(default)]
    pub preset_id: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub lora_id: Option<String>,
    #[serde(default)]
    pub lora_scale: Option<f64>,
    pub duration: i64,
    #[serde(default)]
    pub bpm: Option<i64>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub time_signature: Option<String>,
    #[serde(default)]
    pub seed: Option<i64>,
    #[serde(default)]
    pub instruments: Option<Vec<String>>,
    #[serde(default)]
    pub moods: Option<Vec<String>>,
    #[serde(default = "default_true")]
    pub instrumental: bool,
    #[serde(default)]
    pub params: Option<Map<String, Value>>,
    /// `text` (default) | `reference` | `cover`.
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub reference_audio_id: Option<String>,
    #[serde(default)]
    pub source_audio_id: Option<String>,
    #[serde(default, skip_serializing)]
    pub idempotency_key: Option<String>,
}

fn default_true() -> bool {
    true
}

fn non_empty(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|v| !v.is_empty())
}

/// ACE-Step reads only the first 256 tokens of `[instruction, caption, metas]` (training and generation alike,
/// `conditioning_text.py`). The other parts take about 55 tokens and English runs about 4.5 characters per
/// token (measured with its Qwen3-Embedding tokenizer), so a longer caption pushes bpm / key / duration out.
pub const ACE_CAPTION_CHARS: usize = 850;

/// `text` cut to at most `limit` characters: at the last sentence end if it keeps at least 3/4 of the window,
/// else the last comma, else the last space in the second half (a far-back sentence end must not throw away a
/// quarter or more of the text); else a hard cut.
pub(crate) fn cut_text(text: &str, limit: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let head: String = text.chars().take(limit).collect();
    let after = |floor: usize| move |i: Option<usize>| i.filter(|&i| i >= floor);
    let sentence = after(head.len() * 3 / 4);
    let late = after(head.len() / 2);
    let cut = sentence(
        [". ", "! ", "? "]
            .iter()
            .filter_map(|p| head.rfind(p).map(|i| i + 1))
            .max(),
    )
    .or_else(|| late(head.rfind(", ")))
    .or_else(|| late(head.rfind(' ')))
    .unwrap_or(head.len());
    head[..cut].trim_end().to_string()
}

/// `compose_prompt`, kept within `ACE_CAPTION_CHARS`: the user's prompt is shortened first so the template,
/// instruments, moods and "instrumental" still reach the engine; if those alone are too long, the whole text is cut.
pub fn fit_prompt(
    prompt: &str,
    template: &str,
    instruments: &[String],
    moods: &[String],
    instrumental: bool,
) -> String {
    let compose = |p: &str| compose_prompt(p, template, instruments, moods, instrumental);
    let full = compose(prompt);
    if full.chars().count() <= ACE_CAPTION_CHARS {
        return full;
    }
    // The composed length grows with the prompt's, whatever the template shape ({prompt} once, twice or
    // not at all). Binary-search the longest plain prefix that fits (monotonic in n), then tidy it once with
    // cut_text, which only shortens, so the result still fits.
    let prompt = prompt.trim();
    let prefix = |n: usize| prompt.chars().take(n).collect::<String>();
    let fits = |n: usize| compose(&prefix(n)).chars().count() <= ACE_CAPTION_CHARS;
    let (mut lo, mut hi) = (0, prompt.chars().count());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if fits(mid) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    if lo > 0 {
        compose(&cut_text(prompt, lo))
    } else {
        cut_text(&full, ACE_CAPTION_CHARS)
    }
}

/// Build the caption sent to the engine: template (with optional `{prompt}` slot), then tags.
pub fn compose_prompt(
    prompt: &str,
    template: &str,
    instruments: &[String],
    moods: &[String],
    instrumental: bool,
) -> String {
    let (prompt, template) = (prompt.trim(), template.trim());
    let mut parts: Vec<String> = Vec::new();
    if template.contains("{prompt}") {
        // An empty prompt must not leave a dangling ", " ("keyword, {prompt}" -> "keyword").
        let filled = template.replace("{prompt}", prompt);
        let filled = filled.trim_matches(|c: char| c == ',' || c.is_whitespace());
        if !filled.is_empty() {
            parts.push(filled.to_string());
        }
    } else {
        parts.push(prompt.to_string());
        if !template.is_empty() {
            parts.push(template.to_string());
        }
    }
    parts.extend(
        instruments
            .iter()
            .chain(moods)
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty()),
    );
    if instrumental {
        parts.push("instrumental, no vocals".into());
    }
    parts.join(", ")
}

pub async fn create_generation(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<GenerationRequest>,
) -> AppResult<(StatusCode, Json<Job>)> {
    // A repeated submit (double click, client retry) returns the job that already exists.
    let idempotency_key = non_empty(&req.idempotency_key).map(str::to_string);
    if let Some(key) = &idempotency_key {
        let existing: Option<(String,)> =
            sqlx::query_as("SELECT id FROM jobs WHERE owner_id = ? AND idempotency_key = ?")
                .bind(&user.id)
                .bind(key)
                .fetch_optional(&state.db)
                .await?;
        if let Some((id,)) = existing {
            let job = load_job(&state.db, &id)
                .await?
                .ok_or_else(|| AppError::not_found("job"))?;
            return Ok((StatusCode::OK, Json(job)));
        }
    }

    let built = build_generation(&state, &user, &req).await?;
    let id = insert_job(
        &state,
        &user.id,
        non_empty(&req.project_id),
        &built.title,
        &built.request,
        &built.payload,
        &built.model_id,
        built.lora_id.as_deref(),
        idempotency_key.as_deref(),
    )
    .await?;
    let job = load_job(&state.db, &id)
        .await?
        .ok_or_else(|| AppError::not_found("job"))?;
    Ok((StatusCode::CREATED, Json(job)))
}

/// Everything a `generate` job needs, resolved and validated from a request.
pub(crate) struct BuiltGeneration {
    pub payload: Value,
    pub request: Value,
    pub model_id: String,
    pub lora_id: Option<String>,
    pub title: String,
}

/// Validate a generation request and build the job payload. Shared by the normal generate route and
/// the jobs the server creates itself (LoRA evaluation renders).
pub(crate) async fn build_generation(
    state: &AppState,
    user: &AuthUser,
    req: &GenerationRequest,
) -> AppResult<BuiltGeneration> {
    // Generation mode: which input audio the job needs and which model capability it relies on.
    let mode = non_empty(&req.mode).unwrap_or("text");
    let (input_role, input_id, capability) = match mode {
        "text" => (None, None, "generate"),
        "reference" => (
            Some("reference_audio"),
            non_empty(&req.reference_audio_id),
            "reference_audio",
        ),
        "cover" => (
            Some("source_audio"),
            non_empty(&req.source_audio_id),
            "cover",
        ),
        _ => {
            return Err(AppError::validation(
                "mode phải là text, reference hoặc cover",
            ));
        }
    };
    let unused_input = match mode {
        "text" => {
            non_empty(&req.reference_audio_id).is_some()
                || non_empty(&req.source_audio_id).is_some()
        }
        "reference" => non_empty(&req.source_audio_id).is_some(),
        _ => non_empty(&req.reference_audio_id).is_some(),
    };
    if unused_input {
        return Err(AppError::validation(
            "File âm thanh gửi kèm không khớp với chế độ tạo nhạc đã chọn",
        ));
    }

    let prompt = req.prompt.trim();
    if prompt.is_empty() || prompt.chars().count() > 2000 {
        return Err(AppError::validation("Prompt cần từ 1 đến 2000 ký tự"));
    }
    // A cover takes its length from the Source Audio, so the requested duration is not used.
    if mode != "cover" && !(MIN_DURATION..=MAX_DURATION).contains(&req.duration) {
        return Err(AppError::validation(format!(
            "Duration phải trong khoảng {MIN_DURATION}–{MAX_DURATION} giây"
        )));
    }
    if req.bpm.is_some_and(|b| !(30..=300).contains(&b)) {
        return Err(AppError::validation("BPM phải trong khoảng 30–300"));
    }
    if req
        .seed
        .is_some_and(|s| !(0..=i64::from(i32::MAX)).contains(&s))
    {
        return Err(AppError::validation(
            "Seed phải là số không âm tối đa 2147483647",
        ));
    }

    let preset = match non_empty(&req.preset_id) {
        Some(id) => Some(load_preset(state, id).await?).filter(|p| p.is_active),
        None => None,
    };
    if non_empty(&req.preset_id).is_some() && preset.is_none() {
        return Err(AppError::validation("Preset đã bị tắt"));
    }
    let genre = match non_empty(&req.genre_id) {
        Some(id) => Some(load_genre(state, id).await?),
        None => None,
    };
    let subgenre = match non_empty(&req.subgenre_id) {
        Some(id) => Some(load_genre(state, id).await?),
        None => None,
    };
    if let Some(sub) = &subgenre
        && (sub.parent_id.is_none() || sub.parent_id != genre.as_ref().map(|g| g.id.clone()))
    {
        return Err(AppError::validation("Subgenre không thuộc genre đã chọn"));
    }

    // Model: request overrides preset. The model must exist and be enabled.
    let model_id = non_empty(&req.model_id)
        .map(str::to_string)
        .or_else(|| preset.as_ref().and_then(|p| p.model_id.clone()))
        .ok_or_else(|| AppError::validation("Chưa chọn model"))?;
    let model: (String, String, String, DbJson<Vec<String>>, String) =
        sqlx::query_as("SELECT engine, key, status, capabilities, kind FROM models WHERE id = ?")
            .bind(&model_id)
            .fetch_optional(&state.db)
            .await?
            .ok_or_else(|| AppError::validation("Model không tồn tại"))?;
    if model.4 != "generator" {
        return Err(AppError::validation("Model này không dùng để tạo nhạc"));
    }
    if model.2 != "active" {
        return Err(AppError::validation("Model đã bị tắt"));
    }
    if mode != "text" && !model.3.0.iter().any(|c| c == capability) {
        return Err(AppError::validation(match mode {
            "reference" => "Model đã chọn không hỗ trợ Reference Audio",
            _ => "Model đã chọn không hỗ trợ Cover / Remix",
        }));
    }
    let mut inputs = Map::new();
    if let Some(role) = input_role {
        let id = input_id.ok_or_else(|| {
            AppError::validation(match mode {
                "reference" => "Chế độ Reference to Music cần Reference Audio",
                _ => "Chế độ Cover / Remix cần Source Audio",
            })
        })?;
        let input = load_audio_input(state, user, id)
            .await?
            .ok_or_else(|| AppError::validation("File âm thanh đầu vào không tồn tại"))?;
        inputs.insert(role.into(), input.payload());
    }
    if let Some(strength) = req
        .params
        .as_ref()
        .and_then(|p| p.get("audio_cover_strength"))
        .filter(|v| !v.is_null())
        && !strength.as_f64().is_some_and(|s| (0.0..=1.0).contains(&s))
    {
        return Err(AppError::validation(
            "audio_cover_strength phải trong khoảng 0–1",
        ));
    }

    // LoRA is optional; when present it must be approved and trained on the chosen base model.
    let lora_id = non_empty(&req.lora_id)
        .map(str::to_string)
        .or_else(|| preset.as_ref().and_then(|p| p.lora_id.clone()));
    let lora = match &lora_id {
        Some(id) => {
            let (key, base_model_id, status): (String, String, String) =
                sqlx::query_as("SELECT key, base_model_id, status FROM loras WHERE id = ?")
                    .bind(id)
                    .fetch_optional(&state.db)
                    .await?
                    .ok_or_else(|| AppError::validation("LoRA không tồn tại"))?;
            // An admin may try a LoRA that is still under evaluation; everyone else needs it approved.
            if status != "approved" && !(status == "evaluation" && user.is_admin()) {
                return Err(AppError::validation("LoRA chưa được phê duyệt để sử dụng"));
            }
            if base_model_id != model_id {
                return Err(AppError::validation(
                    "LoRA không tương thích với model đã chọn",
                ));
            }
            let scale = req
                .lora_scale
                .or(preset.as_ref().and_then(|p| p.lora_scale))
                .unwrap_or(1.0);
            if !(0.0..=1.0).contains(&scale) {
                return Err(AppError::validation("LoRA scale phải trong khoảng 0–1"));
            }
            Some(json!({"id": id, "key": key, "scale": scale}))
        }
        None => None,
    };

    let template = [
        preset.as_ref().map(|p| &p.prompt_template),
        subgenre.as_ref().map(|g| &g.prompt_template),
        genre.as_ref().map(|g| &g.prompt_template),
    ]
    .into_iter()
    .flatten()
    .find(|t| !t.trim().is_empty())
    .cloned()
    .unwrap_or_default();
    let instruments = req
        .instruments
        .clone()
        .or_else(|| preset.as_ref().map(|p| p.instruments.0.clone()))
        .unwrap_or_default();
    let moods = req
        .moods
        .clone()
        .or_else(|| preset.as_ref().map(|p| p.moods.0.clone()))
        .unwrap_or_default();
    // What ACE-Step really reads: within ACE_CAPTION_CHARS, so bpm / key / duration are never pushed out.
    let prompt_final = fit_prompt(prompt, &template, &instruments, &moods, req.instrumental);

    // Engine parameters: preset values first, then non-null request values on top.
    let mut params = preset
        .as_ref()
        .and_then(|p| p.params.as_object().cloned())
        .unwrap_or_default();
    for (k, v) in req.params.iter().flatten().filter(|(_, v)| !v.is_null()) {
        params.insert(k.clone(), v.clone());
    }
    if mode != "cover" {
        params.remove("audio_cover_strength");
    }

    let payload = json!({
        "prompt": prompt_final,
        "duration": (mode != "cover").then_some(req.duration),
        "seed": req.seed.unwrap_or_else(random_seed),
        "bpm": req.bpm.or(preset.as_ref().and_then(|p| p.bpm)),
        "key": non_empty(&req.key),
        "time_signature": non_empty(&req.time_signature),
        "model": {"id": model_id, "engine": model.0, "key": model.1},
        "lora": lora,
        "params": params,
        "mode": mode,
        "inputs": inputs,
        "output": {"formats": ["wav", "mp3"]},
    });
    let title = non_empty(&req.title)
        .map(str::to_string)
        .unwrap_or_else(|| prompt.chars().take(60).collect());
    Ok(BuiltGeneration {
        payload,
        request: serde_json::to_value(req)?,
        model_id,
        lora_id,
        title,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_job(
    state: &AppState,
    owner_id: &str,
    project_id: Option<&str>,
    title: &str,
    request: &Value,
    payload: &Value,
    model_id: &str,
    lora_id: Option<&str>,
    idempotency_key: Option<&str>,
) -> AppResult<String> {
    let (id, now) = (new_id(), now());
    sqlx::query(
        "INSERT INTO jobs (id, type, owner_id, project_id, title, request, payload, model_id, lora_id, \
         idempotency_key, created_at, updated_at) VALUES (?, 'generate', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(owner_id)
    .bind(project_id)
    .bind(title)
    .bind(request.to_string())
    .bind(payload.to_string())
    .bind(model_id)
    .bind(lora_id)
    .bind(idempotency_key)
    .bind(&now)
    .bind(&now)
    .execute(&state.db)
    .await?;
    Ok(id)
}

#[derive(Deserialize)]
pub struct JobQuery {
    status: Option<String>,
    #[serde(rename = "type")]
    job_type: Option<String>,
    #[serde(default)]
    mine: bool,
    limit: Option<i64>,
}

pub async fn list_jobs(
    State(state): State<AppState>,
    user: AuthUser,
    Query(q): Query<JobQuery>,
) -> AppResult<Json<Vec<Job>>> {
    let mut sql: QueryBuilder<Sqlite> = QueryBuilder::new(JOB_SELECT);
    sql.push(" WHERE 1 = 1");
    if q.mine || !user.is_admin() {
        sql.push(" AND j.owner_id = ").push_bind(&user.id);
    }
    if let Some(status) = q.status.as_deref().filter(|s| !s.is_empty()) {
        sql.push(" AND j.status = ").push_bind(status);
    }
    if let Some(job_type) = q.job_type.as_deref().filter(|s| !s.is_empty()) {
        // `type=a,b` matches any of the listed types.
        sql.push(" AND j.type IN (");
        let mut types = sql.separated(", ");
        for t in job_type.split(',').map(str::trim).filter(|t| !t.is_empty()) {
            types.push_bind(t.to_string());
        }
        sql.push(")");
    }
    sql.push(" ORDER BY j.created_at DESC, j.id DESC LIMIT ")
        .push_bind(q.limit.unwrap_or(50).clamp(1, 500));
    Ok(Json(sql.build_query_as().fetch_all(&state.db).await?))
}

/// Load a job the caller is allowed to see (owner or admin).
async fn visible_job(state: &AppState, user: &AuthUser, id: &str) -> AppResult<Job> {
    let job = load_job(&state.db, id)
        .await?
        .ok_or_else(|| AppError::not_found("job"))?;
    if !user.can_modify(&job.owner_id) {
        return Err(AppError::not_found("job"));
    }
    Ok(job)
}

#[derive(Serialize)]
pub struct JobDetail {
    #[serde(flatten)]
    job: Job,
    log: String,
}

pub async fn get_job(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<JobDetail>> {
    let job = visible_job(&state, &user, &id).await?;
    let (log,): (String,) = sqlx::query_as("SELECT log FROM jobs WHERE id = ?")
        .bind(&id)
        .fetch_one(&state.db)
        .await?;
    Ok(Json(JobDetail { job, log }))
}

pub async fn cancel_job(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Job>> {
    visible_job(&state, &user, &id).await?;
    let now = now();
    // Queued jobs are cancelled on the spot; in-flight jobs are flagged and the worker confirms.
    let changed = sqlx::query(
        "UPDATE jobs SET \
           status = CASE WHEN status = 'queued' THEN 'cancelled' ELSE status END, \
           finished_at = CASE WHEN status = 'queued' THEN ? ELSE finished_at END, \
           cancel_requested = 1, updated_at = ? \
         WHERE id = ? AND status IN ('queued', 'preparing', 'running')",
    )
    .bind(&now)
    .bind(&now)
    .bind(&id)
    .execute(&state.db)
    .await?;
    if changed.rows_affected() == 0 {
        return Err(AppError::conflict("Job đã kết thúc nên không thể hủy"));
    }
    Ok(Json(visible_job(&state, &user, &id).await?))
}

pub async fn retry_job(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<(StatusCode, Json<Job>)> {
    let job = visible_job(&state, &user, &id).await?;
    if job.job_type != "generate" {
        return Err(AppError::conflict(
            "Chỉ thử lại được job tạo nhạc; training run phải được tạo mới",
        ));
    }
    if job.status != "failed" && job.status != "cancelled" {
        return Err(AppError::conflict(
            "Chỉ thử lại được job đã lỗi hoặc đã hủy",
        ));
    }
    let (payload, project_id, model_id, lora_id): (
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as("SELECT payload, project_id, model_id, lora_id FROM jobs WHERE id = ?")
        .bind(&id)
        .fetch_one(&state.db)
        .await?;
    let model_id = model_id.ok_or_else(|| AppError::conflict("Job không có model để chạy lại"))?;
    let new_id = insert_job(
        &state,
        &job.owner_id,
        project_id.as_deref(),
        &job.title,
        &job.request,
        &serde_json::from_str(&payload)?,
        &model_id,
        lora_id.as_deref(),
        None,
    )
    .await?;
    let new_job = load_job(&state.db, &new_id)
        .await?
        .ok_or_else(|| AppError::not_found("job"))?;
    Ok((StatusCode::CREATED, Json(new_job)))
}

pub async fn queue_summary(
    State(state): State<AppState>,
    _user: AuthUser,
) -> AppResult<Json<Value>> {
    let (queued, preparing, running): (i64, i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(status = 'queued'), 0), COALESCE(SUM(status = 'preparing'), 0), \
         COALESCE(SUM(status = 'running'), 0) FROM jobs WHERE status IN ('queued', 'preparing', 'running')",
    )
    .fetch_one(&state.db)
    .await?;
    let (nodes_online,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM nodes WHERE status = 'active' AND last_seen IS NOT NULL AND last_seen > ?")
            .bind(online_cutoff(&state))
            .fetch_one(&state.db)
            .await?;
    let training = training::active_training(&state).await?;
    Ok(Json(json!({
        "queued": queued, "preparing": preparing, "running": running, "nodes_online": nodes_online,
        "training": training,
    })))
}

#[cfg(test)]
mod tests {
    use super::{ACE_CAPTION_CHARS, cut_text, fit_prompt};

    #[test]
    fn a_short_prompt_is_composed_unchanged() {
        let p = fit_prompt(
            "rainy night",
            "lofi, {prompt}",
            &["piano".into()],
            &["calm".into()],
            true,
        );
        assert_eq!(p, "lofi, rainy night, piano, calm, instrumental, no vocals");
    }

    #[test]
    fn a_long_prompt_is_shortened_so_template_and_tags_survive() {
        let long = "A slow warm lo-fi groove with dusty drums. ".repeat(40);
        let p = fit_prompt(
            &long,
            "lushlofi, {prompt}",
            &["piano".into()],
            &["calm".into()],
            true,
        );
        assert!(
            p.chars().count() <= ACE_CAPTION_CHARS,
            "{}",
            p.chars().count()
        );
        assert!(p.starts_with("lushlofi, A slow warm"));
        assert!(
            p.ends_with("dusty drums., piano, calm, instrumental, no vocals"),
            "{p}"
        );
    }

    #[test]
    fn a_template_using_the_prompt_twice_still_keeps_its_tags() {
        let long = "A slow warm lo-fi groove with dusty drums. ".repeat(40);
        let p = fit_prompt(&long, "{prompt} / {prompt}", &["piano".into()], &[], true);
        assert!(
            p.chars().count() <= ACE_CAPTION_CHARS,
            "{}",
            p.chars().count()
        );
        assert!(p.ends_with(", piano, instrumental, no vocals"), "{p}");
    }

    #[test]
    fn a_long_prompt_keeps_as_much_as_fits_not_half() {
        // Sentence ends early and then far apart: the cut must still keep close to the whole budget.
        let mut long = String::from("Warm intro. ");
        for i in 0..12 {
            long.push_str(&"slow dusty drums and soft keys, ".repeat(3));
            long.push_str(if i % 4 == 3 { "end. " } else { "more " });
        }
        let p = fit_prompt(&long, "", &["piano".into()], &[], true);
        let n = p.chars().count();
        assert!(n <= ACE_CAPTION_CHARS, "{n}");
        assert!(
            n > ACE_CAPTION_CHARS - 120,
            "kept only {n} of {ACE_CAPTION_CHARS}: {p}"
        );
    }

    #[test]
    fn when_the_rest_alone_is_too_long_the_whole_text_is_cut() {
        let moods: Vec<String> = (0..200).map(|i| format!("mood{i}")).collect();
        let p = fit_prompt("rain", "", &[], &moods, false);
        assert!(p.chars().count() <= ACE_CAPTION_CHARS);
        assert!(p.starts_with("rain, mood0"));
    }

    #[test]
    fn cut_text_prefers_sentence_then_comma_then_space_in_the_second_half() {
        assert_eq!(
            cut_text("One two. Three four. Five six seven.", 22),
            "One two. Three four."
        );
        assert_eq!(cut_text("short", 10), "short");
        assert_eq!(cut_text("kw, aaaa aaaa aaaa aaaa", 16), "kw, aaaa aaaa");
        assert_eq!(cut_text("nosentenceendhere", 10), "nosentence");
    }
}
