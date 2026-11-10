//! Model registry, LoRA registry and render node registry (admin side).

use std::collections::HashMap;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, types::Json as DbJson};

use crate::{
    AppState,
    auth::{Admin, AuthUser},
    error::{AppError, AppResult},
    util::{audit, new_id, new_token, now, now_plus, sha256_hex},
};

#[derive(Serialize, FromRow, Clone)]
pub struct InstalledNode {
    #[serde(skip)]
    item_id: String,
    node_id: String,
    node_name: String,
    online: bool,
}

/// Nodes that have each model / LoRA installed, keyed by the model / LoRA id.
async fn installed_nodes(
    state: &AppState,
    table: &str,
    column: &str,
) -> AppResult<HashMap<String, Vec<InstalledNode>>> {
    let rows: Vec<InstalledNode> = sqlx::query_as(&format!(
        "SELECT i.{column} AS item_id, n.id AS node_id, n.name AS node_name, \
         (n.status = 'active' AND n.last_seen IS NOT NULL AND n.last_seen > ?) AS online \
         FROM {table} i JOIN nodes n ON n.id = i.node_id ORDER BY n.name"
    ))
    .bind(online_cutoff(state))
    .fetch_all(&state.db)
    .await?;
    let mut map: HashMap<String, Vec<InstalledNode>> = HashMap::new();
    for row in rows {
        map.entry(row.item_id.clone()).or_default().push(row);
    }
    Ok(map)
}

pub fn online_cutoff(state: &AppState) -> String {
    now_plus(-state.cfg.node_online_seconds)
}

// ---------- models ----------

#[derive(Serialize, FromRow)]
pub struct Model {
    pub id: String,
    pub engine: String,
    pub key: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub capabilities: DbJson<Vec<String>>,
    pub status: String,
    #[sqlx(skip)]
    pub installed_nodes: Vec<InstalledNode>,
}

pub async fn list_models(
    State(state): State<AppState>,
    _user: AuthUser,
) -> AppResult<Json<Vec<Model>>> {
    let mut models: Vec<Model> = sqlx::query_as("SELECT * FROM models ORDER BY engine, name")
        .fetch_all(&state.db)
        .await?;
    let mut nodes = installed_nodes(&state, "model_installations", "model_id").await?;
    for model in &mut models {
        model.installed_nodes = nodes.remove(&model.id).unwrap_or_default();
    }
    Ok(Json(models))
}

#[derive(Deserialize)]
pub struct ModelUpdate {
    name: Option<String>,
    description: Option<String>,
    status: Option<String>,
}

pub async fn update_model(
    State(state): State<AppState>,
    Admin(admin): Admin,
    Path(id): Path<String>,
    Json(req): Json<ModelUpdate>,
) -> AppResult<Json<Model>> {
    if req
        .status
        .as_deref()
        .is_some_and(|s| s != "active" && s != "disabled")
    {
        return Err(AppError::validation("status phải là active hoặc disabled"));
    }
    let updated = sqlx::query(
        "UPDATE models SET name = COALESCE(?, name), description = COALESCE(?, description), \
         status = COALESCE(?, status) WHERE id = ?",
    )
    .bind(&req.name)
    .bind(&req.description)
    .bind(&req.status)
    .bind(&id)
    .execute(&state.db)
    .await?;
    if updated.rows_affected() == 0 {
        return Err(AppError::not_found("model"));
    }
    audit(
        &state.db,
        Some(&admin.id),
        "model.updated",
        "model",
        &id,
        json!({"status": req.status, "name": req.name}),
    )
    .await;
    let mut model: Model = sqlx::query_as("SELECT * FROM models WHERE id = ?")
        .bind(&id)
        .fetch_one(&state.db)
        .await?;
    model.installed_nodes = installed_nodes(&state, "model_installations", "model_id")
        .await?
        .remove(&id)
        .unwrap_or_default();
    Ok(Json(model))
}

// ---------- loras ----------

#[derive(Serialize, FromRow)]
pub struct Lora {
    pub id: String,
    pub key: String,
    pub name: String,
    pub version: i64,
    pub genre_id: Option<String>,
    pub base_model_id: String,
    pub status: String,
    #[sqlx(skip)]
    pub installed_nodes: Vec<InstalledNode>,
}

pub async fn list_loras(
    State(state): State<AppState>,
    _user: AuthUser,
) -> AppResult<Json<Vec<Lora>>> {
    let mut loras: Vec<Lora> = sqlx::query_as("SELECT * FROM loras ORDER BY name, version")
        .fetch_all(&state.db)
        .await?;
    let mut nodes = installed_nodes(&state, "lora_installations", "lora_id").await?;
    for lora in &mut loras {
        lora.installed_nodes = nodes.remove(&lora.id).unwrap_or_default();
    }
    Ok(Json(loras))
}

// ---------- render nodes ----------

#[derive(Serialize, FromRow)]
pub struct Node {
    id: String,
    name: String,
    status: String,
    maintenance: bool,
    online: bool,
    worker_version: Option<String>,
    protocol_version: Option<i64>,
    job_types: DbJson<Vec<String>>,
    engines: DbJson<Value>,
    gpu: DbJson<Value>,
    #[sqlx(skip)]
    models: Vec<Value>,
    #[sqlx(skip)]
    loras: Vec<Value>,
    current_job_id: Option<String>,
    last_seen: Option<String>,
    registered_at: Option<String>,
    created_at: String,
}

async fn load_nodes(state: &AppState, id: Option<&str>) -> AppResult<Vec<Node>> {
    let mut nodes: Vec<Node> = sqlx::query_as(
        "SELECT n.id, n.name, n.status, n.maintenance, \
         (n.status = 'active' AND n.last_seen IS NOT NULL AND n.last_seen > ?) AS online, \
         n.worker_version, n.protocol_version, n.job_types, n.engines, n.gpu, \
         (SELECT j.id FROM jobs j WHERE j.node_id = n.id AND j.status IN ('preparing', 'running') LIMIT 1) AS current_job_id, \
         n.last_seen, n.registered_at, n.created_at \
         FROM nodes n WHERE (? IS NULL OR n.id = ?) ORDER BY n.name",
    )
    .bind(online_cutoff(state))
    .bind(id)
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    let models: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT i.node_id, m.id, m.key, m.engine FROM model_installations i JOIN models m ON m.id = i.model_id ORDER BY m.name",
    )
    .fetch_all(&state.db)
    .await?;
    let loras: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT i.node_id, l.id, l.key FROM lora_installations i JOIN loras l ON l.id = i.lora_id ORDER BY l.name",
    )
    .fetch_all(&state.db)
    .await?;
    for node in &mut nodes {
        node.models = models
            .iter()
            .filter(|m| m.0 == node.id)
            .map(|m| json!({"id": m.1, "key": m.2, "engine": m.3}))
            .collect();
        node.loras = loras
            .iter()
            .filter(|l| l.0 == node.id)
            .map(|l| json!({"id": l.1, "key": l.2}))
            .collect();
    }
    Ok(nodes)
}

async fn load_node(state: &AppState, id: &str) -> AppResult<Node> {
    load_nodes(state, Some(id))
        .await?
        .pop()
        .ok_or_else(|| AppError::not_found("render node"))
}

pub async fn list_nodes(
    State(state): State<AppState>,
    _admin: Admin,
) -> AppResult<Json<Vec<Node>>> {
    Ok(Json(load_nodes(&state, None).await?))
}

fn node_name(name: &str) -> AppResult<&str> {
    let name = name.trim();
    if name.is_empty()
        || name.len() > 64
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(AppError::validation(
            "Tên node chỉ gồm chữ, số, - và _ (tối đa 64 ký tự)",
        ));
    }
    Ok(name)
}

#[derive(Deserialize)]
pub struct NodeCreate {
    name: String,
}

pub async fn create_node(
    State(state): State<AppState>,
    Admin(admin): Admin,
    Json(req): Json<NodeCreate>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let name = node_name(&req.name)?;
    let (id, token) = (new_id(), new_token());
    sqlx::query("INSERT INTO nodes (id, name, token_hash, created_at) VALUES (?, ?, ?, ?)")
        .bind(&id)
        .bind(name)
        .bind(sha256_hex(token.as_bytes()))
        .bind(now())
        .execute(&state.db)
        .await
        .map_err(|e| match AppError::from(e) {
            e if e.status == StatusCode::CONFLICT => AppError::conflict("Tên node đã tồn tại"),
            e => e,
        })?;
    audit(
        &state.db,
        Some(&admin.id),
        "node.created",
        "node",
        &id,
        json!({"name": name}),
    )
    .await;
    Ok((
        StatusCode::CREATED,
        Json(json!({"node": load_node(&state, &id).await?, "token": token})),
    ))
}

#[derive(Deserialize)]
pub struct NodeUpdate {
    name: Option<String>,
    status: Option<String>,
    maintenance: Option<bool>,
}

pub async fn update_node(
    State(state): State<AppState>,
    Admin(admin): Admin,
    Path(id): Path<String>,
    Json(req): Json<NodeUpdate>,
) -> AppResult<Json<Node>> {
    if req
        .status
        .as_deref()
        .is_some_and(|s| s != "active" && s != "disabled")
    {
        return Err(AppError::validation("status phải là active hoặc disabled"));
    }
    let name = req.name.as_deref().map(node_name).transpose()?;
    let updated = sqlx::query(
        "UPDATE nodes SET name = COALESCE(?, name), status = COALESCE(?, status), \
         maintenance = COALESCE(?, maintenance) WHERE id = ?",
    )
    .bind(name)
    .bind(&req.status)
    .bind(req.maintenance)
    .bind(&id)
    .execute(&state.db)
    .await?;
    if updated.rows_affected() == 0 {
        return Err(AppError::not_found("render node"));
    }
    audit(
        &state.db,
        Some(&admin.id),
        "node.updated",
        "node",
        &id,
        json!({"name": name, "status": req.status, "maintenance": req.maintenance}),
    )
    .await;
    Ok(Json(load_node(&state, &id).await?))
}

pub async fn rotate_node_token(
    State(state): State<AppState>,
    Admin(admin): Admin,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let token = new_token();
    let updated = sqlx::query("UPDATE nodes SET token_hash = ? WHERE id = ?")
        .bind(sha256_hex(token.as_bytes()))
        .bind(&id)
        .execute(&state.db)
        .await?;
    if updated.rows_affected() == 0 {
        return Err(AppError::not_found("render node"));
    }
    audit(
        &state.db,
        Some(&admin.id),
        "node.token_rotated",
        "node",
        &id,
        json!({}),
    )
    .await;
    Ok(Json(json!({"token": token})))
}
