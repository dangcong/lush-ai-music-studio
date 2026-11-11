//! Music library and artifact delivery.

use axum::{
    Json,
    body::Body,
    extract::{Path, Query, Request, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{FromRow, QueryBuilder, Sqlite, types::Json as DbJson};
use tokio_util::io::ReaderStream;
use tower::ServiceExt;
use tower_http::services::ServeFile;

use crate::{
    AppState,
    auth::AuthUser,
    error::{AppError, AppResult},
    util::now,
};

const TRACK_SELECT: &str = "SELECT t.id, t.title, t.owner_id, u.username AS owner_name, t.project_id, t.job_id, \
    t.parent_track_id, t.version, t.prompt, t.prompt_final, t.genre_id, g.name AS genre_name, t.subgenre_id, \
    t.preset_id, t.model_id, m.name AS model_name, t.lora_id, t.lora_scale, t.seed, t.bpm, t.key, \
    t.time_signature, t.duration, t.favorite, t.rating, t.params, t.created_at \
    FROM tracks t JOIN users u ON u.id = t.owner_id LEFT JOIN genres g ON g.id = t.genre_id \
    LEFT JOIN models m ON m.id = t.model_id WHERE t.deleted_at IS NULL";

#[derive(Serialize, FromRow)]
pub struct Artifact {
    id: String,
    #[serde(skip)]
    track_id: Option<String>,
    kind: String,
    filename: String,
    content_type: String,
    size_bytes: i64,
    sha256: String,
}

#[derive(Serialize, FromRow)]
pub struct Track {
    id: String,
    title: String,
    owner_id: String,
    owner_name: String,
    project_id: Option<String>,
    job_id: Option<String>,
    parent_track_id: Option<String>,
    version: i64,
    prompt: String,
    prompt_final: String,
    genre_id: Option<String>,
    genre_name: Option<String>,
    subgenre_id: Option<String>,
    preset_id: Option<String>,
    model_id: Option<String>,
    model_name: Option<String>,
    lora_id: Option<String>,
    lora_scale: Option<f64>,
    seed: Option<i64>,
    bpm: Option<i64>,
    key: Option<String>,
    time_signature: Option<String>,
    duration: Option<f64>,
    favorite: bool,
    rating: i64,
    params: DbJson<Value>,
    #[sqlx(skip)]
    artifacts: Vec<Artifact>,
    created_at: String,
}

async fn attach_artifacts(state: &AppState, tracks: &mut [Track]) -> AppResult<()> {
    if tracks.is_empty() {
        return Ok(());
    }
    let mut sql: QueryBuilder<Sqlite> = QueryBuilder::new(
        "SELECT id, track_id, kind, filename, content_type, size_bytes, sha256 FROM artifacts WHERE track_id IN (",
    );
    let mut ids = sql.separated(", ");
    for track in tracks.iter() {
        ids.push_bind(&track.id);
    }
    sql.push(") ORDER BY kind");
    let artifacts: Vec<Artifact> = sql.build_query_as().fetch_all(&state.db).await?;
    for artifact in artifacts {
        if let Some(track) = tracks
            .iter_mut()
            .find(|t| Some(&t.id) == artifact.track_id.as_ref())
        {
            track.artifacts.push(artifact);
        }
    }
    Ok(())
}

async fn load_track(state: &AppState, id: &str) -> AppResult<Track> {
    let track: Track = sqlx::query_as(&format!("{TRACK_SELECT} AND t.id = ?"))
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::not_found("track"))?;
    let mut tracks = [track];
    attach_artifacts(state, &mut tracks).await?;
    let [track] = tracks;
    Ok(track)
}

#[derive(Deserialize)]
pub struct TrackQuery {
    q: Option<String>,
    genre_id: Option<String>,
    project_id: Option<String>,
    #[serde(default)]
    favorite: bool,
    #[serde(default)]
    mine: bool,
    limit: Option<i64>,
}

/// The library is shared: every signed-in employee can browse and play every track.
pub async fn list_tracks(
    State(state): State<AppState>,
    user: AuthUser,
    Query(q): Query<TrackQuery>,
) -> AppResult<Json<Vec<Track>>> {
    let mut sql: QueryBuilder<Sqlite> = QueryBuilder::new(TRACK_SELECT);
    if let Some(text) = q.q.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let like = format!(
            "%{}%",
            text.replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        sql.push(" AND (t.title LIKE ")
            .push_bind(like.clone())
            .push(" ESCAPE '\\' OR t.prompt LIKE ")
            .push_bind(like)
            .push(" ESCAPE '\\')");
    }
    if let Some(genre_id) = q.genre_id.as_deref().filter(|s| !s.is_empty()) {
        sql.push(" AND (t.genre_id = ")
            .push_bind(genre_id)
            .push(" OR t.subgenre_id = ")
            .push_bind(genre_id)
            .push(")");
    }
    if let Some(project_id) = q.project_id.as_deref().filter(|s| !s.is_empty()) {
        sql.push(" AND t.project_id = ").push_bind(project_id);
    }
    if q.favorite {
        sql.push(" AND t.favorite = 1");
    }
    if q.mine {
        sql.push(" AND t.owner_id = ").push_bind(&user.id);
    }
    sql.push(" ORDER BY t.created_at DESC, t.id DESC LIMIT ")
        .push_bind(q.limit.unwrap_or(100).clamp(1, 500));
    let mut tracks: Vec<Track> = sql.build_query_as().fetch_all(&state.db).await?;
    attach_artifacts(&state, &mut tracks).await?;
    Ok(Json(tracks))
}

pub async fn get_track(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Track>> {
    Ok(Json(load_track(&state, &id).await?))
}

#[derive(Deserialize)]
pub struct TrackUpdate {
    title: Option<String>,
    favorite: Option<bool>,
    rating: Option<i64>,
    /// Absent = unchanged, null = remove from project.
    #[serde(default, deserialize_with = "double_option")]
    project_id: Option<Option<String>>,
}

fn double_option<'de, D: serde::Deserializer<'de>>(
    de: D,
) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(de).map(Some)
}

pub async fn update_track(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(req): Json<TrackUpdate>,
) -> AppResult<Json<Track>> {
    let track = load_track(&state, &id).await?;
    if !user.can_modify(&track.owner_id) {
        return Err(AppError::forbidden("Bạn không có quyền sửa track này"));
    }
    let title = req.title.as_deref().map(str::trim);
    if title.is_some_and(|t| t.is_empty() || t.chars().count() > 200) {
        return Err(AppError::validation("Tiêu đề cần từ 1 đến 200 ký tự"));
    }
    if req.rating.is_some_and(|r| !(0..=5).contains(&r)) {
        return Err(AppError::validation("Rating phải từ 0 đến 5"));
    }
    let (set_project, project_id) = match req.project_id {
        Some(value) => (true, value),
        None => (false, None),
    };
    sqlx::query(
        "UPDATE tracks SET title = COALESCE(?, title), favorite = COALESCE(?, favorite), rating = COALESCE(?, rating), \
         project_id = CASE WHEN ? THEN ? ELSE project_id END WHERE id = ?",
    )
    .bind(title)
    .bind(req.favorite)
    .bind(req.rating)
    .bind(set_project)
    .bind(project_id)
    .bind(&id)
    .execute(&state.db)
    .await?;
    Ok(Json(load_track(&state, &id).await?))
}

/// Soft delete: the row and files stay on disk so nothing is lost by mistake.
pub async fn delete_track(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    let track = load_track(&state, &id).await?;
    if !user.can_modify(&track.owner_id) {
        return Err(AppError::forbidden("Bạn không có quyền xóa track này"));
    }
    sqlx::query("UPDATE tracks SET deleted_at = ? WHERE id = ?")
        .bind(now())
        .bind(&id)
        .execute(&state.db)
        .await?;
    crate::util::audit(
        &state.db,
        Some(&user.id),
        "track.deleted",
        "track",
        &id,
        serde_json::json!({"title": track.title}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct ContentQuery {
    #[serde(default)]
    download: bool,
}

/// Stream an artifact by id. `ServeFile` handles `Range` requests so the player can seek.
pub async fn artifact_content(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<String>,
    Query(q): Query<ContentQuery>,
    request: Request,
) -> AppResult<Response> {
    let row: Option<(String, String, String)> = sqlx::query_as(
        "SELECT a.storage_key, a.filename, a.content_type FROM artifacts a \
         LEFT JOIN tracks t ON t.id = a.track_id WHERE a.id = ? AND (a.track_id IS NULL OR t.deleted_at IS NULL)",
    )
    .bind(&id)
    .fetch_optional(&state.db)
    .await?;
    let (storage_key, filename, content_type) = row.ok_or_else(|| AppError::not_found("file"))?;
    let path = state.cfg.data_dir.join(&storage_key);
    if !path.is_file() {
        return Err(AppError::not_found("file"));
    }
    let mime = content_type.parse().map_err(AppError::internal)?;
    let mut response = ServeFile::new_with_mime(path, &mime)
        .oneshot(request)
        .await
        .map_err(AppError::internal)?
        .into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=3600"),
    );
    if q.download
        && let Ok(value) = HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
    {
        response
            .headers_mut()
            .insert(header::CONTENT_DISPOSITION, value);
    }
    Ok(response)
}

/// Every file of a track in one ZIP. Entries are stored, not recompressed (WAV/MP3 barely shrink).
/// The ZIP is built in an anonymous temp file and streamed, so a long WAV is never held in memory;
/// the OS removes the temp file once the response is done with it.
pub async fn track_archive(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Response> {
    sqlx::query_as::<_, (String,)>("SELECT id FROM tracks WHERE id = ? AND deleted_at IS NULL")
        .bind(&id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::not_found("track"))?;
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT storage_key, filename FROM artifacts WHERE track_id = ? ORDER BY kind",
    )
    .bind(&id)
    .fetch_all(&state.db)
    .await?;
    let files: Vec<_> = rows
        .into_iter()
        .map(|(key, name)| (state.cfg.data_dir.join(key), name))
        .filter(|(path, _)| path.is_file())
        .collect();
    if files.is_empty() {
        return Err(AppError::not_found("file"));
    }

    let tmp_dir = state.cfg.data_dir.join("tmp");
    tokio::fs::create_dir_all(&tmp_dir).await?;
    let file = tokio::task::spawn_blocking(move || -> zip::result::ZipResult<std::fs::File> {
        use std::io::{Seek, SeekFrom};
        let mut zip = zip::ZipWriter::new(tempfile::tempfile_in(&tmp_dir)?);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (path, name) in files {
            let mut src = std::fs::File::open(&path)?;
            let large = src.metadata()?.len() >= u64::from(u32::MAX);
            zip.start_file(name, options.large_file(large))?;
            std::io::copy(&mut src, &mut zip)?;
        }
        let mut file = zip.finish()?;
        file.seek(SeekFrom::Start(0))?;
        Ok(file)
    })
    .await
    .map_err(AppError::internal)?
    .map_err(AppError::internal)?;

    let len = file.metadata()?.len();
    let short: String = id
        .chars()
        .rev()
        .take(8)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let body = Body::from_stream(ReaderStream::new(tokio::fs::File::from_std(file)));
    Response::builder()
        .header(header::CONTENT_TYPE, "application/zip")
        .header(header::CONTENT_LENGTH, len)
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"lush-{short}.zip\""),
        )
        .header(header::CACHE_CONTROL, "private, no-store")
        .body(body)
        .map_err(AppError::internal)
}
