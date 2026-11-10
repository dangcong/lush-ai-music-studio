//! Genre registry and style presets. Genres are data: adding one never requires a code change.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, types::Json as DbJson};

use crate::{
    AppState,
    auth::{Admin, AuthUser},
    error::{AppError, AppResult},
    util::{audit, merge, new_id},
};

#[derive(Serialize, Deserialize, FromRow, Clone)]
pub struct Genre {
    pub id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub slug: String,
    pub description: String,
    pub prompt_template: String,
    pub instruments: DbJson<Vec<String>>,
    pub moods: DbJson<Vec<String>>,
    pub bpm_min: Option<i64>,
    pub bpm_max: Option<i64>,
    pub default_duration: i64,
    pub is_active: bool,
    pub sort_order: i64,
}

#[derive(Serialize, Deserialize, FromRow, Clone)]
pub struct Preset {
    pub id: String,
    pub genre_id: String,
    pub name: String,
    pub description: String,
    pub kind: String,
    pub prompt_template: String,
    pub instruments: DbJson<Vec<String>>,
    pub moods: DbJson<Vec<String>>,
    pub bpm: Option<i64>,
    pub duration: Option<i64>,
    pub model_id: Option<String>,
    pub lora_id: Option<String>,
    pub lora_scale: Option<f64>,
    pub params: DbJson<Value>,
    pub version: i64,
    pub is_active: bool,
}

#[derive(Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    include_inactive: bool,
    genre_id: Option<String>,
}

pub async fn load_genre(state: &AppState, id: &str) -> AppResult<Genre> {
    sqlx::query_as("SELECT * FROM genres WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::validation("Genre không tồn tại"))
}

pub async fn load_preset(state: &AppState, id: &str) -> AppResult<Preset> {
    sqlx::query_as("SELECT * FROM style_presets WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::validation("Preset không tồn tại"))
}

// ---------- genres ----------

pub async fn list_genres(
    State(state): State<AppState>,
    user: AuthUser,
    Query(q): Query<ListQuery>,
) -> AppResult<Json<Vec<Genre>>> {
    let all = q.include_inactive && user.is_admin();
    let genres =
        sqlx::query_as("SELECT * FROM genres WHERE (? OR is_active = 1) ORDER BY sort_order, name")
            .bind(all)
            .fetch_all(&state.db)
            .await?;
    Ok(Json(genres))
}

async fn validate_genre(state: &AppState, g: &Genre) -> AppResult<()> {
    if g.name.trim().is_empty() {
        return Err(AppError::validation("Tên genre không được để trống"));
    }
    if g.slug.is_empty()
        || !g
            .slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(AppError::validation(
            "Slug chỉ gồm chữ thường, số và dấu gạch ngang",
        ));
    }
    if !(10..=240).contains(&g.default_duration) {
        return Err(AppError::validation(
            "default_duration phải trong khoảng 10–240 giây",
        ));
    }
    if let (Some(lo), Some(hi)) = (g.bpm_min, g.bpm_max)
        && lo > hi
    {
        return Err(AppError::validation("bpm_min không được lớn hơn bpm_max"));
    }
    if let Some(parent_id) = &g.parent_id {
        if parent_id == &g.id {
            return Err(AppError::validation("Genre không thể là cha của chính nó"));
        }
        if load_genre(state, parent_id).await?.parent_id.is_some() {
            return Err(AppError::validation("Chỉ hỗ trợ 2 cấp: genre và subgenre"));
        }
        let (children,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM genres WHERE parent_id = ?")
            .bind(&g.id)
            .fetch_one(&state.db)
            .await?;
        if children > 0 {
            return Err(AppError::validation(
                "Genre đang có subgenre nên không thể trở thành subgenre",
            ));
        }
    }
    Ok(())
}

async fn save_genre(state: &AppState, g: &Genre) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO genres (id, parent_id, name, slug, description, prompt_template, instruments, moods, bpm_min, \
         bpm_max, default_duration, is_active, sort_order) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT (id) DO UPDATE SET parent_id = excluded.parent_id, name = excluded.name, slug = excluded.slug, \
         description = excluded.description, prompt_template = excluded.prompt_template, \
         instruments = excluded.instruments, moods = excluded.moods, bpm_min = excluded.bpm_min, \
         bpm_max = excluded.bpm_max, default_duration = excluded.default_duration, is_active = excluded.is_active, \
         sort_order = excluded.sort_order",
    )
    .bind(&g.id)
    .bind(&g.parent_id)
    .bind(g.name.trim())
    .bind(&g.slug)
    .bind(&g.description)
    .bind(&g.prompt_template)
    .bind(&g.instruments)
    .bind(&g.moods)
    .bind(g.bpm_min)
    .bind(g.bpm_max)
    .bind(g.default_duration)
    .bind(g.is_active)
    .bind(g.sort_order)
    .execute(&state.db)
    .await
    .map_err(|e| match AppError::from(e) {
        e if e.status == StatusCode::CONFLICT => AppError::conflict("Slug đã được dùng cho genre khác"),
        e => e,
    })?;
    Ok(())
}

pub async fn create_genre(
    State(state): State<AppState>,
    Admin(admin): Admin,
    Json(body): Json<Value>,
) -> AppResult<(StatusCode, Json<Genre>)> {
    let defaults = json!({
        "parent_id": null, "name": "", "slug": "", "description": "", "prompt_template": "",
        "instruments": [], "moods": [], "bpm_min": null, "bpm_max": null,
        "default_duration": 120, "is_active": true, "sort_order": 0,
    });
    let mut value = merge(defaults, &body)?;
    value["id"] = json!(new_id());
    let genre: Genre = serde_json::from_value(value)?;
    validate_genre(&state, &genre).await?;
    save_genre(&state, &genre).await?;
    audit(
        &state.db,
        Some(&admin.id),
        "genre.created",
        "genre",
        &genre.id,
        json!({"slug": genre.slug}),
    )
    .await;
    Ok((StatusCode::CREATED, Json(genre)))
}

pub async fn update_genre(
    State(state): State<AppState>,
    Admin(admin): Admin,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> AppResult<Json<Genre>> {
    let current: Genre = sqlx::query_as("SELECT * FROM genres WHERE id = ?")
        .bind(&id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::not_found("genre"))?;
    let mut value = merge(serde_json::to_value(&current)?, &body)?;
    value["id"] = json!(id);
    let genre: Genre = serde_json::from_value(value)?;
    validate_genre(&state, &genre).await?;
    save_genre(&state, &genre).await?;
    audit(
        &state.db,
        Some(&admin.id),
        "genre.updated",
        "genre",
        &id,
        body,
    )
    .await;
    Ok(Json(genre))
}

// ---------- presets ----------

pub async fn list_presets(
    State(state): State<AppState>,
    user: AuthUser,
    Query(q): Query<ListQuery>,
) -> AppResult<Json<Vec<Preset>>> {
    let all = q.include_inactive && user.is_admin();
    let presets = sqlx::query_as(
        "SELECT * FROM style_presets WHERE (? OR is_active = 1) AND (? IS NULL OR genre_id = ?) ORDER BY name",
    )
    .bind(all)
    .bind(&q.genre_id)
    .bind(&q.genre_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(presets))
}

fn validate_preset(p: &Preset) -> AppResult<()> {
    if p.name.trim().is_empty() {
        return Err(AppError::validation("Tên preset không được để trống"));
    }
    match p.kind.as_str() {
        "genre" => {}
        "trained" if p.lora_id.is_some() => {}
        "trained" => {
            return Err(AppError::validation(
                "Preset loại trained bắt buộc có lora_id",
            ));
        }
        _ => return Err(AppError::validation("kind phải là genre hoặc trained")),
    }
    if p.duration.is_some_and(|d| !(10..=240).contains(&d)) {
        return Err(AppError::validation(
            "duration phải trong khoảng 10–240 giây",
        ));
    }
    if p.lora_scale.is_some_and(|s| !(0.0..=1.0).contains(&s)) {
        return Err(AppError::validation("lora_scale phải trong khoảng 0–1"));
    }
    if !p.params.is_object() {
        return Err(AppError::validation("params phải là JSON object"));
    }
    Ok(())
}

async fn save_preset(state: &AppState, p: &Preset) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO style_presets (id, genre_id, name, description, kind, prompt_template, instruments, moods, bpm, \
         duration, model_id, lora_id, lora_scale, params, version, is_active) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT (id) DO UPDATE SET genre_id = excluded.genre_id, name = excluded.name, \
         description = excluded.description, kind = excluded.kind, prompt_template = excluded.prompt_template, \
         instruments = excluded.instruments, moods = excluded.moods, bpm = excluded.bpm, duration = excluded.duration, \
         model_id = excluded.model_id, lora_id = excluded.lora_id, lora_scale = excluded.lora_scale, \
         params = excluded.params, version = excluded.version, is_active = excluded.is_active",
    )
    .bind(&p.id)
    .bind(&p.genre_id)
    .bind(p.name.trim())
    .bind(&p.description)
    .bind(&p.kind)
    .bind(&p.prompt_template)
    .bind(&p.instruments)
    .bind(&p.moods)
    .bind(p.bpm)
    .bind(p.duration)
    .bind(&p.model_id)
    .bind(&p.lora_id)
    .bind(p.lora_scale)
    .bind(&p.params)
    .bind(p.version)
    .bind(p.is_active)
    .execute(&state.db)
    .await?;
    Ok(())
}

pub async fn create_preset(
    State(state): State<AppState>,
    Admin(admin): Admin,
    Json(body): Json<Value>,
) -> AppResult<(StatusCode, Json<Preset>)> {
    let defaults = json!({
        "genre_id": "", "name": "", "description": "", "kind": "genre", "prompt_template": "",
        "instruments": [], "moods": [], "bpm": null, "duration": null, "model_id": null,
        "lora_id": null, "lora_scale": null, "params": {}, "version": 1, "is_active": true,
    });
    let mut value = merge(defaults, &body)?;
    value["id"] = json!(new_id());
    let preset: Preset = serde_json::from_value(value)?;
    validate_preset(&preset)?;
    save_preset(&state, &preset).await?;
    audit(
        &state.db,
        Some(&admin.id),
        "preset.created",
        "preset",
        &preset.id,
        json!({"name": preset.name}),
    )
    .await;
    Ok((StatusCode::CREATED, Json(preset)))
}

pub async fn update_preset(
    State(state): State<AppState>,
    Admin(admin): Admin,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> AppResult<Json<Preset>> {
    let current: Preset = sqlx::query_as("SELECT * FROM style_presets WHERE id = ?")
        .bind(&id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::not_found("preset"))?;
    let mut value = merge(serde_json::to_value(&current)?, &body)?;
    value["id"] = json!(id);
    let preset: Preset = serde_json::from_value(value)?;
    validate_preset(&preset)?;
    save_preset(&state, &preset).await?;
    audit(
        &state.db,
        Some(&admin.id),
        "preset.updated",
        "preset",
        &id,
        body,
    )
    .await;
    Ok(Json(preset))
}
