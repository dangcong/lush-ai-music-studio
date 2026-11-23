//! Catalogs: named collections of uploaded songs used as LoRA training material.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::FromRow;

use crate::{
    AppState,
    auth::Trainer,
    error::{AppError, AppResult},
    util::{audit, new_id, now},
};

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
