//! Catalogs: named collections of uploaded songs used as LoRA training material.

use axum::{
    Json,
    extract::{Multipart, Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, SqlitePool};
use tokio::fs;

use crate::{
    AppState,
    auth::Trainer,
    error::{AppError, AppResult},
    routes::inputs::{self, StoredAudio},
    util::{audit, new_id, now},
};

pub const ANALYZE_JOB_TYPE: &str = "analyze_track";
/// Analysis runs after generation (priority 0) and before captioning (-20).
const ANALYZE_PRIORITY: i64 = -10;

const MAX_NAME_CHARS: usize = 120;

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

/// One song in a catalog. `captioning_model_key` stays `null` until the captioning job exists.
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
    fn into_view(self, caption: Option<CaptionView>, models_done: Vec<String>) -> CatalogTrack {
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
            captioning_model_key: None,
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
    let rows = sqlx::query_as::<_, TrackRow>(&format!(
        "{SELECT_TRACK} AND t.catalog_id = ? ORDER BY t.created_at ASC, t.id ASC"
    ))
    .bind(&catalog_id)
    .fetch_all(&state.db)
    .await?;
    let mut views = Vec::with_capacity(rows.len());
    for row in rows {
        views.push(hydrate(&state.db, row).await?);
    }
    Ok(Json(views))
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

    let (track_id, job_id, ts) = (new_id(), new_id(), now());
    let payload = json!({
        "catalog_track_id": track_id,
        "inputs": {"track": {
            "artifact_id": stored.artifact_id, "filename": stored.filename,
            "size_bytes": stored.size_bytes, "sha256": stored.sha256,
        }},
    });
    let inserted: Result<(), sqlx::Error> = async {
        let mut tx = state.db.begin().await?;
        sqlx::query(
            "INSERT INTO jobs (id, type, priority, owner_id, title, payload, max_attempts, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, 3, ?, ?)",
        )
        .bind(&job_id)
        .bind(ANALYZE_JOB_TYPE)
        .bind(ANALYZE_PRIORITY)
        .bind(&user.id)
        .bind(&stored.filename)
        .bind(payload.to_string())
        .bind(&ts)
        .bind(&ts)
        .execute(&mut *tx)
        .await?;
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

/// The revision the head of `(track, model_key)` points at, with undo/redo availability.
async fn head_caption(
    db: &SqlitePool,
    track_id: &str,
    model_key: &str,
) -> Result<Option<CaptionView>, sqlx::Error> {
    sqlx::query_as::<_, CaptionView>(
        "SELECT r.model_key, r.seq, r.mood, r.instruments, r.vocals, r.caption, r.source, r.source_ref, \
                r.created_at, \
                EXISTS (SELECT 1 FROM caption_revisions p WHERE p.catalog_track_id = r.catalog_track_id \
                          AND p.model_key = r.model_key AND p.seq < r.seq) AS can_undo, \
                EXISTS (SELECT 1 FROM caption_revisions n WHERE n.catalog_track_id = r.catalog_track_id \
                          AND n.model_key = r.model_key AND n.seq > r.seq) AS can_redo \
         FROM caption_heads h \
         JOIN caption_revisions r ON r.catalog_track_id = h.catalog_track_id \
              AND r.model_key = h.model_key AND r.seq = h.current_seq \
         WHERE h.catalog_track_id = ? AND h.model_key = ?",
    )
    .bind(track_id)
    .bind(model_key)
    .fetch_optional(db)
    .await
}

async fn hydrate(db: &SqlitePool, row: TrackRow) -> AppResult<CatalogTrack> {
    let caption = match &row.current_model_key {
        Some(key) => head_caption(db, &row.id, key).await?,
        None => None,
    };
    let done: Vec<(String,)> = sqlx::query_as(
        "SELECT model_key FROM caption_heads WHERE catalog_track_id = ? ORDER BY model_key",
    )
    .bind(&row.id)
    .fetch_all(db)
    .await?;
    Ok(row.into_view(caption, done.into_iter().map(|(k,)| k).collect()))
}

/// A live track (not soft-deleted, in a live catalog) as the API view; 404 otherwise.
async fn fetch_track(state: &AppState, id: &str) -> AppResult<CatalogTrack> {
    let row = sqlx::query_as::<_, TrackRow>(&format!(
        "{SELECT_TRACK} AND t.id = ? AND t.catalog_id IN (SELECT id FROM catalogs WHERE deleted_at IS NULL)"
    ))
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::not_found("bài hát"))?;
    hydrate(&state.db, row).await
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
    mood: String,
    instruments: String,
    vocals: String,
    caption: String,
}

/// Append a human revision (`seq = max + 1`) and point the head at it, in one transaction. A concurrent
/// save that takes the same `seq` loses on the UNIQUE index; retry once, then answer 409.
async fn insert_human_revision(
    state: &AppState,
    user_id: &str,
    track_id: &str,
    model_key: &str,
    body: &CaptionBody,
) -> AppResult<()> {
    for attempt in 0..2 {
        let rev_id = new_id();
        let result: Result<(), sqlx::Error> = async {
            let mut tx = state.db.begin().await?;
            sqlx::query(
                "INSERT INTO caption_revisions \
                   (id, catalog_track_id, model_key, seq, mood, instruments, vocals, caption, source, created_by, created_at) \
                 SELECT ?, ?, ?, COALESCE(MAX(seq), 0) + 1, ?, ?, ?, ?, 'human', ?, ? \
                 FROM caption_revisions WHERE catalog_track_id = ? AND model_key = ?",
            )
            .bind(&rev_id)
            .bind(track_id)
            .bind(model_key)
            .bind(&body.mood)
            .bind(&body.instruments)
            .bind(&body.vocals)
            .bind(&body.caption)
            .bind(user_id)
            .bind(now())
            .bind(track_id)
            .bind(model_key)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO caption_heads (catalog_track_id, model_key, current_seq) \
                 VALUES (?, ?, (SELECT seq FROM caption_revisions WHERE id = ?)) \
                 ON CONFLICT (catalog_track_id, model_key) DO UPDATE SET current_seq = excluded.current_seq",
            )
            .bind(track_id)
            .bind(model_key)
            .bind(&rev_id)
            .execute(&mut *tx)
            .await?;
            tx.commit().await
        }
        .await;
        match result {
            Ok(()) => return Ok(()),
            Err(sqlx::Error::Database(db)) if db.is_unique_violation() && attempt == 0 => {}
            Err(sqlx::Error::Database(db)) if db.is_unique_violation() => {
                return Err(AppError::conflict("Mô tả vừa được sửa ở nơi khác, thử lại"));
            }
            Err(err) => return Err(AppError::internal(err)),
        }
    }
    Err(AppError::conflict("Mô tả vừa được sửa ở nơi khác, thử lại"))
}

pub async fn put_caption(
    State(state): State<AppState>,
    Trainer(user): Trainer,
    Path((id, model_key)): Path<(String, String)>,
    Json(body): Json<CaptionBody>,
) -> AppResult<Json<CaptionView>> {
    ensure_track(&state, &id).await?;
    ensure_captioner(&state, &model_key).await?;
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
    let (job_id, ts) = (new_id(), now());
    let payload = json!({
        "catalog_track_id": id,
        "inputs": {"track": {
            "artifact_id": artifact_id, "filename": filename,
            "size_bytes": size_bytes, "sha256": sha256,
        }},
    });
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "INSERT INTO jobs (id, type, priority, owner_id, title, payload, max_attempts, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, 3, ?, ?)",
    )
    .bind(&job_id)
    .bind(ANALYZE_JOB_TYPE)
    .bind(ANALYZE_PRIORITY)
    .bind(&user.id)
    .bind(&filename)
    .bind(payload.to_string())
    .bind(&ts)
    .bind(&ts)
    .execute(&mut *tx)
    .await?;
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
