//! Catalogs: named collections of uploaded songs used as LoRA training material.

use axum::{
    Json,
    extract::{Multipart, Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::FromRow;
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

/// One song in a catalog. The caption fields are filled by the captioning tasks; until then they are
/// `null` / `[]` / `false`.
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
    pub caption: Option<Value>,
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
    fn into_view(self) -> CatalogTrack {
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
            models_done: Vec::new(),
            caption: None,
            captioning_model_key: None,
            voice_warning: false,
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
    Ok(Json(rows.into_iter().map(TrackRow::into_view).collect()))
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
    let row = sqlx::query_as::<_, TrackRow>(&format!("{SELECT_TRACK} AND t.id = ?"))
        .bind(&track_id)
        .fetch_one(&state.db)
        .await?;
    Ok((StatusCode::CREATED, Json(row.into_view())))
}
