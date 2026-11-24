//! Catalogs: named collections of uploaded songs used as LoRA training material.

use axum::{
    Json,
    extract::{Multipart, Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, SqliteConnection, SqlitePool};
use std::collections::HashMap;
use tokio::fs;

use crate::{
    AppState,
    auth::Trainer,
    error::{AppError, AppResult},
    routes::inputs::{self, StoredAudio},
    util::{audit, new_id, now},
};

pub const ANALYZE_JOB_TYPE: &str = "analyze_track";
pub const CAPTION_JOB_TYPE: &str = "caption_tracks";
/// Analysis runs after generation (priority 0) and before captioning (-20).
const ANALYZE_PRIORITY: i64 = -10;
const CAPTION_PRIORITY: i64 = -20;

const MAX_NAME_CHARS: usize = 120;
const MAX_TAG_CHARS: usize = 500;
const MAX_CAPTION_CHARS: usize = 4000;

#[derive(Serialize, FromRow)]
pub struct Catalog {
    pub id: String,
    pub name: String,
    pub genre_id: String,
    pub genre_name: String,
    pub track_count: i64,
    pub total_seconds: f64,
    pub created_at: String,
}

#[derive(Deserialize)]
pub struct CreateCatalog {
    name: String,
    genre_id: String,
}

#[derive(Deserialize)]
pub struct UpdateCatalog {
    name: Option<String>,
}

const SELECT_CATALOG: &str = "SELECT c.id, c.name, c.genre_id, g.name AS genre_name, \
     (SELECT COUNT(*) FROM catalog_tracks t WHERE t.catalog_id = c.id AND t.deleted_at IS NULL) AS track_count, \
     (SELECT COALESCE(SUM(t.duration_s), 0.0) FROM catalog_tracks t WHERE t.catalog_id = c.id AND t.deleted_at IS NULL) AS total_seconds, \
     c.created_at \
     FROM catalogs c JOIN genres g ON g.id = c.genre_id WHERE c.deleted_at IS NULL";

fn clean_name(raw: &str) -> AppResult<String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(AppError::validation("Tên catalog không được để trống"));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(AppError::validation(format!(
            "Tên catalog tối đa {MAX_NAME_CHARS} ký tự"
        )));
    }
    Ok(name.to_string())
}

fn name_conflict(err: sqlx::Error) -> AppError {
    match &err {
        sqlx::Error::Database(db) if db.is_unique_violation() => {
            AppError::conflict("Đã có catalog trùng tên")
        }
        _ => AppError::internal(err),
    }
}

async fn fetch(state: &AppState, id: &str) -> AppResult<Catalog> {
    sqlx::query_as::<_, Catalog>(&format!("{SELECT_CATALOG} AND c.id = ?"))
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .map_err(AppError::internal)?
        .ok_or_else(|| AppError::not_found("catalog"))
}

pub async fn list_catalogs(
    State(state): State<AppState>,
    _trainer: Trainer,
) -> AppResult<Json<Vec<Catalog>>> {
    let rows = sqlx::query_as::<_, Catalog>(&format!(
        "{SELECT_CATALOG} ORDER BY c.created_at DESC, c.id DESC"
    ))
    .fetch_all(&state.db)
    .await
    .map_err(AppError::internal)?;
    Ok(Json(rows))
}

pub async fn create_catalog(
    State(state): State<AppState>,
    Trainer(user): Trainer,
    Json(body): Json<CreateCatalog>,
) -> AppResult<(StatusCode, Json<Catalog>)> {
    let name = clean_name(&body.name)?;
    let genre: Option<(String,)> = sqlx::query_as("SELECT id FROM genres WHERE id = ?")
        .bind(&body.genre_id)
        .fetch_optional(&state.db)
        .await
        .map_err(AppError::internal)?;
    if genre.is_none() {
        return Err(AppError::validation("Thể loại không tồn tại"));
    }
    let id = new_id();
    let ts = now();
    sqlx::query(
        "INSERT INTO catalogs (id, name, genre_id, created_by, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&name)
    .bind(&body.genre_id)
    .bind(&user.id)
    .bind(&ts)
    .bind(&ts)
    .execute(&state.db)
    .await
    .map_err(name_conflict)?;
    audit(
        &state.db,
        Some(&user.id),
        "catalog.created",
        "catalog",
        &id,
        json!({"name": name, "genre_id": body.genre_id}),
    )
    .await;
    Ok((StatusCode::CREATED, Json(fetch(&state, &id).await?)))
}

pub async fn get_catalog(
    State(state): State<AppState>,
    _trainer: Trainer,
    Path(id): Path<String>,
) -> AppResult<Json<Catalog>> {
    Ok(Json(fetch(&state, &id).await?))
}

pub async fn update_catalog(
    State(state): State<AppState>,
    Trainer(user): Trainer,
    Path(id): Path<String>,
    Json(body): Json<UpdateCatalog>,
) -> AppResult<Json<Catalog>> {
    let name = body.name.as_deref().map(clean_name).transpose()?;
    let result = sqlx::query(
        "UPDATE catalogs SET name = COALESCE(?, name), updated_at = ? \
         WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(&name)
    .bind(now())
    .bind(&id)
    .execute(&state.db)
    .await
    .map_err(name_conflict)?;
    if result.rows_affected() == 0 {
        return Err(AppError::not_found("catalog"));
    }
    audit(
        &state.db,
        Some(&user.id),
        "catalog.updated",
        "catalog",
        &id,
        json!({"name": name}),
    )
    .await;
    Ok(Json(fetch(&state, &id).await?))
}

pub async fn delete_catalog(
    State(state): State<AppState>,
    Trainer(user): Trainer,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    let ts = now();
    let result = sqlx::query(
        "UPDATE catalogs SET deleted_at = ?, updated_at = ? WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(&ts)
    .bind(&ts)
    .bind(&id)
    .execute(&state.db)
    .await
    .map_err(AppError::internal)?;
    if result.rows_affected() == 0 {
        return Err(AppError::not_found("catalog"));
    }
    audit(
        &state.db,
        Some(&user.id),
        "catalog.deleted",
        "catalog",
        &id,
        json!({}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// The stored audio of a catalog track as a worker input (`payload.inputs.<name>`).
pub struct TrackAudio {
    pub artifact_id: String,
    pub filename: String,
    pub size_bytes: i64,
    pub sha256: String,
}

impl TrackAudio {
    fn input(&self) -> Value {
        json!({
            "artifact_id": self.artifact_id, "filename": self.filename,
            "size_bytes": self.size_bytes, "sha256": self.sha256,
        })
    }
}

/// Insert a queued `analyze_track` job for a track and return its id. The caller links it to the track
/// (`catalog_tracks.analysis_job_id`) inside the same transaction.
async fn enqueue_analysis(
    tx: &mut SqliteConnection,
    owner_id: &str,
    track_id: &str,
    audio: &TrackAudio,
    ts: &str,
) -> Result<String, sqlx::Error> {
    let job_id = new_id();
    let payload = json!({"catalog_track_id": track_id, "inputs": {"track": audio.input()}});
    sqlx::query(
        "INSERT INTO jobs (id, type, priority, owner_id, title, payload, max_attempts, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, 3, ?, ?)",
    )
    .bind(&job_id)
    .bind(ANALYZE_JOB_TYPE)
    .bind(ANALYZE_PRIORITY)
    .bind(owner_id)
    .bind(&audio.filename)
    .bind(payload.to_string())
    .bind(ts)
    .bind(ts)
    .execute(tx)
    .await?;
    Ok(job_id)
}

// ---------- tracks ----------

#[derive(Serialize)]
pub struct TempoEstimate {
    pub bpm: Option<f64>,
    pub confidence: f64,
    pub alternatives: Vec<f64>,
}

#[derive(Serialize)]
pub struct KeyEstimate {
    pub key: Option<String>,
    pub confidence: f64,
}

#[derive(Serialize)]
pub struct Estimates {
    pub tempo: Option<TempoEstimate>,
    pub key: Option<KeyEstimate>,
}

/// One song in a catalog. `captioning_model_key` is the model of a live `caption_tracks` job that has not
/// produced this song's result yet.
#[derive(Serialize)]
pub struct CatalogTrack {
    pub id: String,
    pub catalog_id: String,
    pub artifact_id: String,
    pub filename: String,
    pub duration_s: Option<f64>,
    pub tempo_bpm: Option<f64>,
    pub tempo_source: Option<String>,
    pub key_name: Option<String>,
    pub key_source: Option<String>,
    pub analysis_status: String,
    pub estimates: Estimates,
    pub current_model_key: Option<String>,
    pub models_done: Vec<String>,
    pub caption: Option<CaptionView>,
    pub captioning_model_key: Option<String>,
    pub voice_warning: bool,
    pub created_at: String,
}

#[derive(FromRow)]
struct TrackRow {
    id: String,
    catalog_id: String,
    artifact_id: String,
    filename: String,
    duration_s: Option<f64>,
    tempo_bpm: Option<f64>,
    tempo_source: Option<String>,
    key_name: Option<String>,
    key_source: Option<String>,
    analysis: String,
    analysis_status: String,
    current_model_key: Option<String>,
    created_at: String,
}

impl TrackRow {
    fn into_view(
        self,
        caption: Option<CaptionView>,
        models_done: Vec<String>,
        captioning_model_key: Option<String>,
    ) -> CatalogTrack {
        let analysis: Value = serde_json::from_str(&self.analysis).unwrap_or_default();
        let tempo = analysis["tempo"].is_object().then(|| TempoEstimate {
            bpm: analysis["tempo"]["bpm"].as_f64(),
            confidence: analysis["tempo"]["confidence"].as_f64().unwrap_or(0.0),
            alternatives: analysis["tempo"]["alternatives"]
                .as_array()
                .map(|a| a.iter().filter_map(Value::as_f64).collect())
                .unwrap_or_default(),
        });
        let key = analysis["key"].is_object().then(|| KeyEstimate {
            key: analysis["key"]["key"].as_str().map(String::from),
            confidence: analysis["key"]["confidence"].as_f64().unwrap_or(0.0),
        });
        CatalogTrack {
            id: self.id,
            catalog_id: self.catalog_id,
            artifact_id: self.artifact_id,
            filename: self.filename,
            duration_s: self.duration_s,
            tempo_bpm: self.tempo_bpm,
            tempo_source: self.tempo_source,
            key_name: self.key_name,
            key_source: self.key_source,
            analysis_status: self.analysis_status,
            estimates: Estimates { tempo, key },
            current_model_key: self.current_model_key,
            models_done,
            voice_warning: caption
                .as_ref()
                .is_some_and(|c| voice_warning(&c.vocals, &c.caption)),
            caption,
            captioning_model_key,
            created_at: self.created_at,
        }
    }
}

/// `analysis_status` 'failed' is not stored: it is derived from the analysis job (failed or cancelled),
/// which also covers a job that ran out of attempts through lease expiry.
const SELECT_TRACK: &str = "SELECT t.id, t.catalog_id, t.artifact_id, t.filename, t.duration_s, t.tempo_bpm, \
     t.tempo_source, t.key_name, t.key_source, t.analysis, \
     CASE WHEN t.analysis_status = 'pending' AND j.status IN ('failed', 'cancelled') THEN 'failed' \
          ELSE t.analysis_status END AS analysis_status, \
     t.current_model_key, t.created_at \
     FROM catalog_tracks t LEFT JOIN jobs j ON j.id = t.analysis_job_id \
     WHERE t.deleted_at IS NULL";

async fn ensure_catalog(state: &AppState, id: &str) -> AppResult<()> {
    let found: Option<(String,)> =
        sqlx::query_as("SELECT id FROM catalogs WHERE id = ? AND deleted_at IS NULL")
            .bind(id)
            .fetch_optional(&state.db)
            .await?;
    found
        .map(|_| ())
        .ok_or_else(|| AppError::not_found("catalog"))
}

pub async fn list_tracks(
    State(state): State<AppState>,
    _trainer: Trainer,
    Path(catalog_id): Path<String>,
) -> AppResult<Json<Vec<CatalogTrack>>> {
    ensure_catalog(&state, &catalog_id).await?;
    Ok(Json(
        load_tracks(&state.db, Scope::Catalog(&catalog_id)).await?,
    ))
}

/// Drop the artifact (and its file) of an upload that was rejected after it had been stored.
async fn discard_upload(state: &AppState, stored: &StoredAudio) {
    let key: Option<(String,)> = sqlx::query_as("SELECT storage_key FROM artifacts WHERE id = ?")
        .bind(&stored.artifact_id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
    let _ = sqlx::query("DELETE FROM artifacts WHERE id = ?")
        .bind(&stored.artifact_id)
        .execute(&state.db)
        .await;
    if let Some((key,)) = key {
        let _ = fs::remove_file(state.cfg.data_dir.join(key)).await;
    }
}

async fn duplicate_error(state: &AppState, catalog_id: &str, sha256: &str) -> AppError {
    let existing: Option<(String,)> = sqlx::query_as(
        "SELECT filename FROM catalog_tracks WHERE catalog_id = ? AND sha256 = ? AND deleted_at IS NULL",
    )
    .bind(catalog_id)
    .bind(sha256)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    match existing {
        Some((name,)) => AppError::conflict(format!("File này đã có trong catalog: {name}")),
        None => AppError::conflict("File này đã có trong catalog"),
    }
}

pub async fn upload_track(
    State(state): State<AppState>,
    Trainer(user): Trainer,
    Path(catalog_id): Path<String>,
    multipart: Multipart,
) -> AppResult<(StatusCode, Json<CatalogTrack>)> {
    ensure_catalog(&state, &catalog_id).await?;
    let stored = inputs::store_audio_upload(&state, &user, multipart).await?;

    let live: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM catalog_tracks WHERE catalog_id = ? AND sha256 = ? AND deleted_at IS NULL",
    )
    .bind(&catalog_id)
    .bind(&stored.sha256)
    .fetch_optional(&state.db)
    .await?;
    if live.is_some() {
        let err = duplicate_error(&state, &catalog_id, &stored.sha256).await;
        discard_upload(&state, &stored).await;
        return Err(err);
    }

    let (track_id, ts) = (new_id(), now());
    let inserted: Result<(), sqlx::Error> = async {
        let mut tx = state.db.begin().await?;
        let audio = TrackAudio {
            artifact_id: stored.artifact_id.clone(),
            filename: stored.filename.clone(),
            size_bytes: stored.size_bytes,
            sha256: stored.sha256.clone(),
        };
        let job_id = enqueue_analysis(&mut tx, &user.id, &track_id, &audio, &ts).await?;
        sqlx::query(
            "INSERT INTO catalog_tracks (id, catalog_id, artifact_id, filename, sha256, analysis_job_id, created_by, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&track_id)
        .bind(&catalog_id)
        .bind(&stored.artifact_id)
        .bind(&stored.filename)
        .bind(&stored.sha256)
        .bind(&job_id)
        .bind(&user.id)
        .bind(&ts)
        .execute(&mut *tx)
        .await?;
        tx.commit().await
    }
    .await;
    if let Err(err) = inserted {
        // A concurrent upload of the same file wins the unique index.
        let unique = matches!(&err, sqlx::Error::Database(db) if db.is_unique_violation());
        let result = if unique {
            duplicate_error(&state, &catalog_id, &stored.sha256).await
        } else {
            AppError::internal(err)
        };
        discard_upload(&state, &stored).await;
        return Err(result);
    }
    audit(
        &state.db,
        Some(&user.id),
        "catalog_track.uploaded",
        "catalog_track",
        &track_id,
        json!({"catalog_id": catalog_id, "filename": stored.filename}),
    )
    .await;
    Ok((
        StatusCode::CREATED,
        Json(fetch_track(&state, &track_id).await?),
    ))
}

// ---------- captions and track edits ----------

/// Words in a caption (or a non-empty `vocals` field) that suggest a human voice on the track.
pub const VOICE_TERMS: &[&str] = &[
    "vocal", "vocals", "voice", "singing", "singer", "sung", "hum", "humming", "chant", "spoken",
    "speech", "rap", "choir", "lyrics",
];

/// True when the caption says there is a voice: `vocals` is anything but empty / "instrumental", or the
/// caption contains one of [`VOICE_TERMS`] as a whole word (case-insensitive).
pub fn voice_warning(vocals: &str, caption: &str) -> bool {
    let vocals = vocals.trim();
    if !vocals.is_empty() && !vocals.eq_ignore_ascii_case("instrumental") {
        return true;
    }
    caption
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .any(|w| VOICE_TERMS.iter().any(|t| w.eq_ignore_ascii_case(t)))
}

#[derive(Serialize, FromRow)]
pub struct CaptionView {
    pub model_key: String,
    pub seq: i64,
    pub mood: String,
    pub instruments: String,
    pub vocals: String,
    pub caption: String,
    pub source: String,
    pub source_ref: String,
    pub created_at: String,
    pub can_undo: bool,
    pub can_redo: bool,
}

/// A caption head's revision plus undo/redo availability; the caller appends the join to
/// `catalog_tracks` / the `WHERE` clause that selects which heads it wants.
const HEAD_SELECT: &str = "SELECT h.catalog_track_id, r.model_key, r.seq, r.mood, r.instruments, r.vocals, r.caption, \
            r.source, r.source_ref, r.created_at, \
            EXISTS (SELECT 1 FROM caption_revisions p WHERE p.catalog_track_id = r.catalog_track_id \
                      AND p.model_key = r.model_key AND p.seq < r.seq) AS can_undo, \
            EXISTS (SELECT 1 FROM caption_revisions n WHERE n.catalog_track_id = r.catalog_track_id \
                      AND n.model_key = r.model_key AND n.seq > r.seq) AS can_redo \
     FROM caption_heads h \
     JOIN caption_revisions r ON r.catalog_track_id = h.catalog_track_id \
          AND r.model_key = h.model_key AND r.seq = h.current_seq";

#[derive(FromRow)]
struct HeadRow {
    catalog_track_id: String,
    #[sqlx(flatten)]
    view: CaptionView,
}

/// The revision the head of `(track, model_key)` points at, with undo/redo availability.
async fn head_caption(
    db: &SqlitePool,
    track_id: &str,
    model_key: &str,
) -> Result<Option<CaptionView>, sqlx::Error> {
    let row = sqlx::query_as::<_, HeadRow>(&format!(
        "{HEAD_SELECT} WHERE h.catalog_track_id = ? AND h.model_key = ?"
    ))
    .bind(track_id)
    .bind(model_key)
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| r.view))
}

/// Which tracks [`load_tracks`] returns.
pub(crate) enum Scope<'a> {
    /// Every live track of a catalog.
    Catalog(&'a str),
    /// One live track in a live catalog.
    Track(&'a str),
}

impl Scope<'_> {
    fn clause(&self) -> &'static str {
        match self {
            Scope::Catalog(_) => "t.catalog_id = ?",
            Scope::Track(_) => {
                "t.id = ? AND t.catalog_id IN (SELECT id FROM catalogs WHERE deleted_at IS NULL)"
            }
        }
    }

    fn id(&self) -> &str {
        match self {
            Scope::Catalog(id) | Scope::Track(id) => id,
        }
    }
}

/// Condition on a live `caption_tracks` job `j` and one of its payload tracks `e` (a `json_each` row): the job has no
/// revision for that track yet. A finished or cancelled job, and a track it already answered, no longer count as
/// "being captioned". Each job's tracks are expanded once (`FROM jobs j, json_each(...) e`), not once per track.
const CAPTION_PENDING: &str = "j.type = 'caption_tracks' AND j.status IN ('queued', 'preparing', 'running') \
     AND NOT EXISTS (SELECT 1 FROM caption_revisions r \
                     WHERE r.job_id = j.id AND r.catalog_track_id = json_extract(e.value, '$.catalog_track_id'))";

/// Tracks of `scope` as API views, oldest first. The caption heads, `models_done` and in-flight caption
/// jobs come from one query each for the whole scope, grouped here, so the cost does not grow per track.
pub(crate) async fn load_tracks(db: &SqlitePool, scope: Scope<'_>) -> AppResult<Vec<CatalogTrack>> {
    let (clause, id) = (scope.clause(), scope.id());
    let rows = sqlx::query_as::<_, TrackRow>(&format!(
        "{SELECT_TRACK} AND {clause} ORDER BY t.created_at ASC, t.id ASC"
    ))
    .bind(id)
    .fetch_all(db)
    .await?;

    let mut heads: HashMap<String, CaptionView> = sqlx::query_as::<_, HeadRow>(&format!(
        "{HEAD_SELECT} JOIN catalog_tracks t ON t.id = h.catalog_track_id AND t.current_model_key = h.model_key \
         WHERE t.deleted_at IS NULL AND {clause}"
    ))
    .bind(id)
    .fetch_all(db)
    .await?
    .into_iter()
    .map(|r| (r.catalog_track_id, r.view))
    .collect();

    let mut done: HashMap<String, Vec<String>> = HashMap::new();
    let done_rows: Vec<(String, String)> = sqlx::query_as(&format!(
        "SELECT h.catalog_track_id, h.model_key FROM caption_heads h \
         JOIN catalog_tracks t ON t.id = h.catalog_track_id \
         WHERE t.deleted_at IS NULL AND {clause} ORDER BY h.model_key"
    ))
    .bind(id)
    .fetch_all(db)
    .await?;
    for (track_id, model_key) in done_rows {
        done.entry(track_id).or_default().push(model_key);
    }

    let mut captioning: HashMap<String, String> = HashMap::new();
    let pending: Vec<(String, Option<String>)> = sqlx::query_as(&format!(
        "SELECT t.id, json_extract(j.payload, '$.model.key') \
         FROM jobs j, json_each(j.payload, '$.tracks') e \
         JOIN catalog_tracks t ON t.id = json_extract(e.value, '$.catalog_track_id') \
         WHERE {CAPTION_PENDING} AND t.deleted_at IS NULL AND {clause} ORDER BY j.created_at ASC, j.id ASC"
    ))
    .bind(id)
    .fetch_all(db)
    .await?;
    for (track_id, model_key) in pending {
        if let Some(model_key) = model_key {
            captioning.entry(track_id).or_insert(model_key); // the oldest live job wins
        }
    }

    Ok(rows
        .into_iter()
        .map(|row| {
            let (caption, done, captioning) = (
                heads.remove(&row.id),
                done.remove(&row.id).unwrap_or_default(),
                captioning.remove(&row.id),
            );
            row.into_view(caption, done, captioning)
        })
        .collect())
}

/// A live track (not soft-deleted, in a live catalog) as the API view; 404 otherwise.
async fn fetch_track(state: &AppState, id: &str) -> AppResult<CatalogTrack> {
    load_tracks(&state.db, Scope::Track(id))
        .await?
        .pop()
        .ok_or_else(|| AppError::not_found("bài hát"))
}

async fn ensure_track(state: &AppState, id: &str) -> AppResult<()> {
    let found: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM catalog_tracks WHERE id = ? AND deleted_at IS NULL \
           AND catalog_id IN (SELECT id FROM catalogs WHERE deleted_at IS NULL)",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    found
        .map(|_| ())
        .ok_or_else(|| AppError::not_found("bài hát"))
}

async fn ensure_captioner(state: &AppState, key: &str) -> AppResult<()> {
    let found: Option<(String,)> =
        sqlx::query_as("SELECT id FROM models WHERE key = ? AND kind = 'captioner' LIMIT 1")
            .bind(key)
            .fetch_optional(&state.db)
            .await?;
    found
        .map(|_| ())
        .ok_or_else(|| AppError::validation("Model mô tả không tồn tại"))
}

/// Distinguishes a missing field (`None`) from an explicit `null` (`Some(None)`).
fn nullable<'de, T: Deserialize<'de>, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(d).map(Some)
}

#[derive(Deserialize)]
pub struct UpdateTrack {
    #[serde(default, deserialize_with = "nullable")]
    tempo_bpm: Option<Option<f64>>,
    #[serde(default, deserialize_with = "nullable")]
    key_name: Option<Option<String>>,
    current_model_key: Option<String>,
}

pub async fn update_track(
    State(state): State<AppState>,
    Trainer(user): Trainer,
    Path(id): Path<String>,
    Json(body): Json<UpdateTrack>,
) -> AppResult<Json<CatalogTrack>> {
    ensure_track(&state, &id).await?;
    if let Some(Some(bpm)) = body.tempo_bpm
        && !(20.0..=300.0).contains(&bpm)
    {
        return Err(AppError::validation("Tempo phải từ 20 đến 300 BPM"));
    }
    let key = match &body.key_name {
        Some(Some(raw)) => {
            let key = raw.trim();
            if key.is_empty() || key.chars().count() > 20 {
                return Err(AppError::validation("Key phải có 1 đến 20 ký tự"));
            }
            Some(Some(key.to_string()))
        }
        Some(None) => Some(None),
        None => None,
    };
    if let Some(model_key) = &body.current_model_key {
        ensure_captioner(&state, model_key).await?;
    }
    let mut sets: Vec<&str> = Vec::new();
    if body.tempo_bpm.is_some() {
        sets.push("tempo_bpm = ?, tempo_source = 'human'");
    }
    if key.is_some() {
        sets.push("key_name = ?, key_source = 'human'");
    }
    if body.current_model_key.is_some() {
        sets.push("current_model_key = ?");
    }
    if !sets.is_empty() {
        let sql = format!(
            "UPDATE catalog_tracks SET {} WHERE id = ? AND deleted_at IS NULL",
            sets.join(", ")
        );
        let mut query = sqlx::query(&sql);
        if let Some(bpm) = body.tempo_bpm {
            query = query.bind(bpm);
        }
        if let Some(key) = &key {
            query = query.bind(key.clone());
        }
        if let Some(model_key) = &body.current_model_key {
            query = query.bind(model_key);
        }
        let done = query.bind(&id).execute(&state.db).await?;
        if done.rows_affected() == 0 {
            return Err(AppError::not_found("bài hát"));
        }
        audit(
            &state.db,
            Some(&user.id),
            "catalog_track.updated",
            "catalog_track",
            &id,
            json!({
                "tempo_bpm": body.tempo_bpm, "key_name": key,
                "current_model_key": body.current_model_key,
            }),
        )
        .await;
    }
    Ok(Json(fetch_track(&state, &id).await?))
}

pub async fn delete_track(
    State(state): State<AppState>,
    Trainer(user): Trainer,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    let result = sqlx::query(
        "UPDATE catalog_tracks SET deleted_at = ? WHERE id = ? AND deleted_at IS NULL \
           AND catalog_id IN (SELECT id FROM catalogs WHERE deleted_at IS NULL)",
    )
    .bind(now())
    .bind(&id)
    .execute(&state.db)
    .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::not_found("bài hát"));
    }
    audit(
        &state.db,
        Some(&user.id),
        "catalog_track.deleted",
        "catalog_track",
        &id,
        json!({}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct CaptionBody {
    pub mood: String,
    pub instruments: String,
    pub vocals: String,
    pub caption: String,
}

/// One caption revision to append. `source` is `human` (website edit) or `model` (a `caption_tracks` job).
pub(crate) struct NewRevision<'a> {
    pub track_id: &'a str,
    pub model_key: &'a str,
    pub body: &'a CaptionBody,
    pub source: &'a str,
    pub source_ref: &'a str,
    pub job_id: Option<&'a str>,
    pub created_by: &'a str,
}

/// Append a revision (`seq = max + 1`) and point the head at it, in one transaction; returns its `seq`.
/// A concurrent save that takes the same `seq` loses on the UNIQUE index; retry once, then answer 409.
/// With a `job_id`, a revision that job already wrote for the track is returned as is (idempotent retry):
/// nothing is added and the head does not move.
pub(crate) async fn insert_revision(db: &SqlitePool, rev: &NewRevision<'_>) -> AppResult<i64> {
    for attempt in 0..2 {
        let rev_id = new_id();
        let result: Result<i64, sqlx::Error> = async {
            let mut tx = db.begin().await?;
            sqlx::query(
                "INSERT INTO caption_revisions \
                   (id, catalog_track_id, model_key, seq, mood, instruments, vocals, caption, source, source_ref, \
                    job_id, created_by, created_at) \
                 SELECT ?, ?, ?, COALESCE(MAX(seq), 0) + 1, ?, ?, ?, ?, ?, ?, ?, ?, ? \
                 FROM caption_revisions WHERE catalog_track_id = ? AND model_key = ?",
            )
            .bind(&rev_id)
            .bind(rev.track_id)
            .bind(rev.model_key)
            .bind(&rev.body.mood)
            .bind(&rev.body.instruments)
            .bind(&rev.body.vocals)
            .bind(&rev.body.caption)
            .bind(rev.source)
            .bind(rev.source_ref)
            .bind(rev.job_id)
            .bind(rev.created_by)
            .bind(now())
            .bind(rev.track_id)
            .bind(rev.model_key)
            .execute(&mut *tx)
            .await?;
            let (seq,): (i64,) = sqlx::query_as(
                "INSERT INTO caption_heads (catalog_track_id, model_key, current_seq) \
                 VALUES (?, ?, (SELECT seq FROM caption_revisions WHERE id = ?)) \
                 ON CONFLICT (catalog_track_id, model_key) DO UPDATE SET current_seq = excluded.current_seq \
                 RETURNING current_seq",
            )
            .bind(rev.track_id)
            .bind(rev.model_key)
            .bind(&rev_id)
            .fetch_one(&mut *tx)
            .await?;
            tx.commit().await?;
            Ok(seq)
        }
        .await;
        match result {
            Ok(seq) => return Ok(seq),
            Err(sqlx::Error::Database(db_err)) if db_err.is_unique_violation() => {
                if let Some(job_id) = rev.job_id {
                    let existing: Option<(i64,)> = sqlx::query_as(
                        "SELECT seq FROM caption_revisions WHERE job_id = ? AND catalog_track_id = ?",
                    )
                    .bind(job_id)
                    .bind(rev.track_id)
                    .fetch_optional(db)
                    .await?;
                    if let Some((seq,)) = existing {
                        return Ok(seq);
                    }
                }
                if attempt == 1 {
                    return Err(AppError::conflict("Mô tả vừa được sửa ở nơi khác, thử lại"));
                }
            }
            Err(err) => return Err(AppError::internal(err)),
        }
    }
    Err(AppError::conflict("Mô tả vừa được sửa ở nơi khác, thử lại"))
}

async fn insert_human_revision(
    state: &AppState,
    user_id: &str,
    track_id: &str,
    model_key: &str,
    body: &CaptionBody,
) -> AppResult<()> {
    insert_revision(
        &state.db,
        &NewRevision {
            track_id,
            model_key,
            body,
            source: "human",
            source_ref: "",
            job_id: None,
            created_by: user_id,
        },
    )
    .await
    .map(|_| ())
}

pub async fn put_caption(
    State(state): State<AppState>,
    Trainer(user): Trainer,
    Path((id, model_key)): Path<(String, String)>,
    Json(body): Json<CaptionBody>,
) -> AppResult<Json<CaptionView>> {
    ensure_track(&state, &id).await?;
    ensure_captioner(&state, &model_key).await?;
    for (label, text, max) in [
        ("Mood", &body.mood, MAX_TAG_CHARS),
        ("Nhạc cụ", &body.instruments, MAX_TAG_CHARS),
        ("Giọng hát", &body.vocals, MAX_TAG_CHARS),
        ("Mô tả", &body.caption, MAX_CAPTION_CHARS),
    ] {
        if text.chars().count() > max {
            return Err(AppError::validation(format!("{label} tối đa {max} ký tự")));
        }
    }
    insert_human_revision(&state, &user.id, &id, &model_key, &body).await?;
    audit(
        &state.db,
        Some(&user.id),
        "catalog_track.caption_edited",
        "catalog_track",
        &id,
        json!({"model_key": model_key}),
    )
    .await;
    let view = head_caption(&state.db, &id, &model_key)
        .await?
        .ok_or_else(|| AppError::not_found("mô tả"))?;
    Ok(Json(view))
}

async fn move_head(
    state: &AppState,
    id: &str,
    model_key: &str,
    older: bool,
) -> AppResult<Json<CaptionView>> {
    ensure_track(state, id).await?;
    let (cmp, agg) = if older { ("<", "MAX") } else { (">", "MIN") };
    let target = format!(
        "SELECT {agg}(r.seq) FROM caption_revisions r WHERE r.catalog_track_id = caption_heads.catalog_track_id \
           AND r.model_key = caption_heads.model_key AND r.seq {cmp} caption_heads.current_seq"
    );
    let moved = sqlx::query(&format!(
        "UPDATE caption_heads SET current_seq = ({target}) \
         WHERE catalog_track_id = ? AND model_key = ? AND ({target}) IS NOT NULL"
    ))
    .bind(id)
    .bind(model_key)
    .execute(&state.db)
    .await?;
    if moved.rows_affected() == 0 {
        let has_head = head_caption(&state.db, id, model_key).await?.is_some();
        return Err(match (has_head, older) {
            (false, _) => AppError::not_found("mô tả"),
            (true, true) => AppError::conflict("Không còn bản trước"),
            (true, false) => AppError::conflict("Không còn bản sau"),
        });
    }
    let view = head_caption(&state.db, id, model_key)
        .await?
        .ok_or_else(|| AppError::not_found("mô tả"))?;
    Ok(Json(view))
}

pub async fn undo_caption(
    State(state): State<AppState>,
    _trainer: Trainer,
    Path((id, model_key)): Path<(String, String)>,
) -> AppResult<Json<CaptionView>> {
    move_head(&state, &id, &model_key, true).await
}

pub async fn redo_caption(
    State(state): State<AppState>,
    _trainer: Trainer,
    Path((id, model_key)): Path<(String, String)>,
) -> AppResult<Json<CaptionView>> {
    move_head(&state, &id, &model_key, false).await
}

/// Queue a fresh `analyze_track` job (the "Thử lại" button). Refused while one is still queued or running.
pub async fn reanalyze_track(
    State(state): State<AppState>,
    Trainer(user): Trainer,
    Path(id): Path<String>,
) -> AppResult<Json<CatalogTrack>> {
    ensure_track(&state, &id).await?;
    let (filename, sha256, artifact_id, size_bytes): (String, String, String, i64) =
        sqlx::query_as(
            "SELECT t.filename, t.sha256, t.artifact_id, a.size_bytes \
         FROM catalog_tracks t JOIN artifacts a ON a.id = t.artifact_id WHERE t.id = ?",
        )
        .bind(&id)
        .fetch_one(&state.db)
        .await?;
    let ts = now();
    let audio = TrackAudio {
        artifact_id,
        filename,
        size_bytes,
        sha256,
    };
    let mut tx = state.db.begin().await?;
    let job_id = enqueue_analysis(&mut tx, &user.id, &id, &audio, &ts).await?;
    let switched = sqlx::query(
        "UPDATE catalog_tracks SET analysis_job_id = ?, analysis_status = 'pending' \
         WHERE id = ? AND deleted_at IS NULL AND NOT EXISTS ( \
           SELECT 1 FROM jobs j WHERE j.id = catalog_tracks.analysis_job_id \
             AND j.status IN ('queued', 'preparing', 'running'))",
    )
    .bind(&job_id)
    .bind(&id)
    .execute(&mut *tx)
    .await?;
    if switched.rows_affected() == 0 {
        tx.rollback().await?;
        return Err(AppError::conflict("Bài này đang được đo"));
    }
    tx.commit().await?;
    audit(
        &state.db,
        Some(&user.id),
        "catalog_track.reanalyzed",
        "catalog_track",
        &id,
        json!({"job_id": job_id}),
    )
    .await;
    Ok(Json(fetch_track(&state, &id).await?))
}

// ---------- caption jobs ----------

#[derive(Deserialize)]
pub struct CaptionJobBody {
    model_key: String,
    track_ids: Vec<String>,
    #[serde(default)]
    force: bool,
}

#[derive(FromRow)]
struct CaptionCandidate {
    id: String,
    filename: String,
    sha256: String,
    artifact_id: String,
    size_bytes: i64,
    has_head: bool,
    pending: bool,
}

/// Queue one `caption_tracks` job for the tracks that need a caption from `model_key`. Without `force`,
/// tracks that already have a head for the model, or wait for a live job of the same model, are skipped.
/// Nothing to queue means no job (`job_id: null`).
pub async fn create_caption_job(
    State(state): State<AppState>,
    Trainer(user): Trainer,
    Path(catalog_id): Path<String>,
    Json(body): Json<CaptionJobBody>,
) -> AppResult<Json<Value>> {
    ensure_catalog(&state, &catalog_id).await?;
    let model: Option<(String, String)> = sqlx::query_as(
        "SELECT id, name FROM models WHERE key = ? AND kind = 'captioner' AND status = 'active' LIMIT 1",
    )
    .bind(&body.model_key)
    .fetch_optional(&state.db)
    .await?;
    let (model_id, model_name) =
        model.ok_or_else(|| AppError::validation("Model mô tả không tồn tại"))?;
    let installed: Option<(i64,)> = sqlx::query_as(
        "SELECT 1 FROM model_installations mi JOIN nodes n ON n.id = mi.node_id \
             WHERE mi.model_id = ? AND n.status = 'active' LIMIT 1",
    )
    .bind(&model_id)
    .fetch_optional(&state.db)
    .await?;
    if installed.is_none() {
        return Err(AppError::validation("Model chưa cài trên máy render"));
    }

    let mut tx = state.db.begin().await?;
    let candidates: Vec<CaptionCandidate> = sqlx::query_as(&format!(
        "SELECT t.id, t.filename, t.sha256, t.artifact_id, a.size_bytes, \
             EXISTS (SELECT 1 FROM caption_heads h WHERE h.catalog_track_id = t.id AND h.model_key = ?1) AS has_head, \
             EXISTS (SELECT 1 FROM jobs j, json_each(j.payload, '$.tracks') e \
                     WHERE json_extract(e.value, '$.catalog_track_id') = t.id AND {CAPTION_PENDING} \
                     AND json_extract(j.payload, '$.model.key') = ?1) AS pending \
             FROM catalog_tracks t JOIN artifacts a ON a.id = t.artifact_id \
             WHERE t.catalog_id = ?2 AND t.deleted_at IS NULL"
    ))
    .bind(&body.model_key)
    .bind(&catalog_id)
    .fetch_all(&mut *tx)
    .await?;
    let by_id: HashMap<&str, &CaptionCandidate> =
        candidates.iter().map(|c| (c.id.as_str(), c)).collect();

    let (mut queued, mut skipped): (Vec<&CaptionCandidate>, Vec<String>) = (Vec::new(), Vec::new());
    for track_id in &body.track_ids {
        let Some(candidate) = by_id.get(track_id.as_str()) else {
            return Err(AppError::validation(
                "Có bài không thuộc catalog này hoặc đã bị xóa",
            ));
        };
        if queued.iter().any(|q| q.id == *track_id) || skipped.contains(track_id) {
            continue; // listed twice
        }
        if !body.force && (candidate.has_head || candidate.pending) {
            skipped.push(track_id.clone());
        } else {
            queued.push(candidate);
        }
    }
    if queued.is_empty() {
        return Ok(Json(
            json!({"job_id": null, "queued": [], "skipped": skipped}),
        ));
    }

    let (job_id, ts) = (new_id(), now());
    let mut tracks = Vec::with_capacity(queued.len());
    let mut inputs = serde_json::Map::new();
    for (i, c) in queued.iter().enumerate() {
        let name = format!("t{i}");
        let audio = TrackAudio {
            artifact_id: c.artifact_id.clone(),
            filename: c.filename.clone(),
            size_bytes: c.size_bytes,
            sha256: c.sha256.clone(),
        };
        tracks.push(json!({"catalog_track_id": c.id, "input": name}));
        inputs.insert(name, audio.input());
    }
    let payload = json!({
        "catalog_id": catalog_id,
        "model": {"id": model_id, "key": body.model_key},
        "tracks": tracks,
        "inputs": inputs,
    });
    sqlx::query(
        "INSERT INTO jobs (id, type, priority, owner_id, title, payload, model_id, max_attempts, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, 2, ?, ?)",
    )
    .bind(&job_id)
    .bind(CAPTION_JOB_TYPE)
    .bind(CAPTION_PRIORITY)
    .bind(&user.id)
    .bind(format!("Mô tả {} bài · {model_name}", queued.len()))
    .bind(payload.to_string())
    .bind(&model_id)
    .bind(&ts)
    .bind(&ts)
    .execute(&mut *tx)
    .await?;
    // The model the user chose for this run is the one the Tech stack column shows.
    for c in &queued {
        sqlx::query("UPDATE catalog_tracks SET current_model_key = ? WHERE id = ?")
            .bind(&body.model_key)
            .bind(&c.id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    let queued_ids: Vec<&str> = queued.iter().map(|c| c.id.as_str()).collect();
    audit(
        &state.db,
        Some(&user.id),
        "catalog.caption_job_created",
        "catalog",
        &catalog_id,
        json!({"job_id": job_id, "model_key": body.model_key, "tracks": queued_ids, "force": body.force}),
    )
    .await;
    Ok(Json(
        json!({"job_id": job_id, "queued": queued_ids, "skipped": skipped}),
    ))
}

#[cfg(test)]
mod tests {
    use super::voice_warning;

    #[test]
    fn voice_warning_matches_whole_words_only() {
        assert!(!voice_warning("Instrumental", "warm pads"));
        assert!(!voice_warning("", "humanized drums"));
        assert!(voice_warning("", "soft hummed vocal line"));
        assert!(voice_warning("", "Soft HUMMING, then rap."));
        assert!(voice_warning("female voice", ""));
        assert!(!voice_warning(" INSTRUMENTAL ", "grapes and therapy"));
    }
}
