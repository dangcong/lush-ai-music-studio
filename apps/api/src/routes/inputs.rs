//! Input audio uploaded by users (Reference Audio / Source Audio) and its delivery to the render node.
//! A file has no role of its own: the job's `mode` decides how the engine uses it.

use axum::{
    Json,
    extract::{Multipart, Path, Query, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::FromRow;
use tokio::{fs, io::AsyncWriteExt};
use tower::ServiceExt;
use tower_http::services::ServeFile;

use crate::{
    AppState,
    auth::{AuthUser, NodeAuth},
    error::{AppError, AppResult},
    routes::worker::{TempFile, lease_lost},
    util::{audit, new_id, now},
};

pub const KIND: &str = "audio_input";

#[derive(Serialize, FromRow)]
pub struct AudioInput {
    pub id: String,
    #[serde(skip)]
    pub owner_id: Option<String>,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub sha256: String,
    pub created_at: String,
}

impl AudioInput {
    /// What a job payload (and later the track metadata) records about an input file.
    pub fn payload(&self) -> Value {
        json!({"artifact_id": self.id, "filename": self.filename, "size_bytes": self.size_bytes, "sha256": self.sha256})
    }
}

/// Accepted formats, decided by the extension AND the first bytes of the file.
fn audio_format(file_name: &str, head: &[u8]) -> Option<(&'static str, &'static str)> {
    let ext = file_name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "wav" if head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WAVE" => {
            Some(("wav", "audio/wav"))
        }
        "flac" if head.starts_with(b"fLaC") => Some(("flac", "audio/flac")),
        // MP3: an ID3v2 tag or an MPEG audio frame sync.
        "mp3"
            if head.starts_with(b"ID3")
                || (head.len() >= 2 && head[0] == 0xFF && head[1] & 0xE0 == 0xE0) =>
        {
            Some(("mp3", "audio/mpeg"))
        }
        _ => None,
    }
}

/// Keep the user's file name for display only: no path, no control characters, bounded length.
fn display_name(client_name: &str, ext: &str) -> String {
    let base = client_name.rsplit(['/', '\\']).next().unwrap_or_default();
    let stem = base.rsplit_once('.').map(|(s, _)| s).unwrap_or(base);
    let stem: String = stem
        .chars()
        .filter(|c| !c.is_control() && *c != '"')
        .take(120)
        .collect();
    let stem = stem.trim();
    format!("{}.{ext}", if stem.is_empty() { "audio" } else { stem })
}

pub async fn upload_audio_input(
    State(state): State<AppState>,
    user: AuthUser,
    mut multipart: Multipart,
) -> AppResult<(StatusCode, Json<AudioInput>)> {
    let limit = (state.cfg.max_input_audio_mb * 1024 * 1024) as i64;
    let tmp_dir = state.cfg.data_dir.join("tmp");
    fs::create_dir_all(&tmp_dir).await?;

    let mut upload: Option<(TempFile, String, Vec<u8>, String, i64)> = None; // file, name, head, sha, size
    while let Some(mut field) = multipart.next_field().await? {
        if field.name() != Some("file") {
            continue;
        }
        let client_name = field.file_name().unwrap_or_default().to_string();
        let path = tmp_dir.join(new_id());
        let guard = TempFile(Some(path.clone()));
        let mut file = fs::File::create(&path).await?;
        let (mut hasher, mut size, mut head) = (Sha256::new(), 0i64, Vec::with_capacity(12));
        while let Some(chunk) = field.chunk().await? {
            size += chunk.len() as i64;
            if size > limit {
                return Err(too_large(state.cfg.max_input_audio_mb));
            }
            if head.len() < 12 {
                head.extend(chunk.iter().take(12 - head.len()));
            }
            hasher.update(&chunk);
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        upload = Some((
            guard,
            client_name,
            head,
            hex::encode(hasher.finalize()),
            size,
        ));
        break;
    }
    let (mut temp, client_name, head, sha256, size) =
        upload.ok_or_else(|| AppError::bad_request("Thiếu field 'file'"))?;
    if size == 0 {
        return Err(AppError::validation("File rỗng"));
    }
    let (ext, content_type) = audio_format(&client_name, &head)
        .ok_or_else(|| AppError::validation("Chỉ nhận file âm thanh WAV, MP3 hoặc FLAC hợp lệ"))?;

    // Storage keys are built only from server-side ids, never from client input.
    let id = new_id();
    let storage_key = format!("inputs/{id}.{ext}");
    let final_path = state.cfg.data_dir.join(&storage_key);
    fs::create_dir_all(final_path.parent().expect("storage key has a parent")).await?;
    let source = temp.0.take().expect("temp file present");
    if let Err(err) = fs::rename(&source, &final_path).await {
        temp.0 = Some(source);
        return Err(err.into());
    }
    let input = AudioInput {
        id,
        owner_id: Some(user.id.clone()),
        filename: display_name(&client_name, ext),
        content_type: content_type.into(),
        size_bytes: size,
        sha256,
        created_at: now(),
    };
    let inserted = sqlx::query(
        "INSERT INTO artifacts (id, kind, owner_id, storage_key, filename, content_type, size_bytes, sha256, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&input.id)
    .bind(KIND)
    .bind(&user.id)
    .bind(&storage_key)
    .bind(&input.filename)
    .bind(&input.content_type)
    .bind(input.size_bytes)
    .bind(&input.sha256)
    .bind(&input.created_at)
    .execute(&state.db)
    .await;
    if let Err(err) = inserted {
        let _ = fs::remove_file(&final_path).await;
        return Err(err.into());
    }
    audit(
        &state.db,
        Some(&user.id),
        "audio_input.uploaded",
        "artifact",
        &input.id,
        json!({"filename": input.filename, "size_bytes": input.size_bytes}),
    )
    .await;
    Ok((StatusCode::CREATED, Json(input)))
}

pub fn too_large(limit_mb: usize) -> AppError {
    AppError::new(
        StatusCode::PAYLOAD_TOO_LARGE,
        "payload_too_large",
        format!("File vượt quá {limit_mb} MB"),
    )
}

/// An input file the caller may use: their own, or any when the caller is an admin.
pub async fn load_audio_input(
    state: &AppState,
    user: &AuthUser,
    id: &str,
) -> AppResult<Option<AudioInput>> {
    let input: Option<AudioInput> = sqlx::query_as(
        "SELECT id, owner_id, filename, content_type, size_bytes, sha256, created_at FROM artifacts \
         WHERE id = ? AND kind = ?",
    )
    .bind(id)
    .bind(KIND)
    .fetch_optional(&state.db)
    .await?;
    Ok(input.filter(|i| i.owner_id.as_deref().is_some_and(|o| user.can_modify(o))))
}

pub async fn get_audio_input(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<AudioInput>> {
    load_audio_input(&state, &user, &id)
        .await?
        .map(Json)
        .ok_or_else(|| AppError::not_found("audio input"))
}

#[derive(Deserialize)]
pub struct LeaseQuery {
    lease_token: String,
}

/// Worker protocol: stream one input file of a job to the node that currently holds that job.
pub async fn job_input(
    State(state): State<AppState>,
    node: NodeAuth,
    Path((job_id, artifact_id)): Path<(String, String)>,
    Query(q): Query<LeaseQuery>,
    request: Request,
) -> AppResult<Response> {
    let held: Option<(String,)> = sqlx::query_as(
        "SELECT payload FROM jobs WHERE id = ? AND node_id = ? AND lease_token = ? AND status IN ('preparing', 'running')",
    )
    .bind(&job_id)
    .bind(&node.id)
    .bind(&q.lease_token)
    .fetch_optional(&state.db)
    .await?;
    let (payload,) = held.ok_or_else(lease_lost)?;
    let payload: Value = serde_json::from_str(&payload)?;
    let listed = payload["inputs"].as_object().is_some_and(|inputs| {
        inputs
            .values()
            .any(|i| i["artifact_id"].as_str() == Some(artifact_id.as_str()))
    });
    if !listed {
        return Err(AppError::not_found("input"));
    }
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT storage_key, content_type FROM artifacts WHERE id = ? AND kind = ?")
            .bind(&artifact_id)
            .bind(KIND)
            .fetch_optional(&state.db)
            .await?;
    let (storage_key, content_type) = row.ok_or_else(|| AppError::not_found("input"))?;
    let path = state.cfg.data_dir.join(storage_key);
    if !path.is_file() {
        return Err(AppError::not_found("input"));
    }
    let mime = content_type.parse().map_err(AppError::internal)?;
    Ok(ServeFile::new_with_mime(path, &mime)
        .oneshot(request)
        .await
        .map_err(AppError::internal)?
        .into_response())
}
