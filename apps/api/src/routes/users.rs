//! Auth, users, projects and audit log endpoints.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, types::Json as DbJson};

use crate::{
    AppState,
    auth::{Admin, AuthUser, SESSION_COOKIE},
    error::{AppError, AppResult},
    util::{
        audit, hash_password, new_id, new_token, now, now_plus, sha256_hex, validate_password,
        verify_password,
    },
};

const ROLES: [&str; 3] = ["admin", "trainer", "member"];
const USER_COLUMNS: &str = "id, username, display_name, role, is_active, created_at";

#[derive(Serialize, FromRow)]
pub struct User {
    id: String,
    username: String,
    display_name: String,
    role: String,
    is_active: bool,
    created_at: String,
}

async fn load_user(state: &AppState, id: &str) -> AppResult<User> {
    sqlx::query_as(&format!("SELECT {USER_COLUMNS} FROM users WHERE id = ?"))
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::not_found("người dùng"))
}

fn session_cookie(state: &AppState, token: &str, max_age: i64) -> HeaderValue {
    let secure = if state.cfg.cookie_secure {
        "; Secure"
    } else {
        ""
    };
    HeaderValue::from_str(&format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}"
    ))
    .expect("cookie is ASCII")
}

// ---------- auth ----------

#[derive(Deserialize)]
pub struct LoginRequest {
    username: String,
    password: String,
}

pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> AppResult<Response> {
    let row: Option<(String, String, bool)> =
        sqlx::query_as("SELECT id, password_hash, is_active FROM users WHERE username = ?")
            .bind(req.username.trim())
            .fetch_optional(&state.db)
            .await?;
    let invalid = || {
        AppError::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Sai tên đăng nhập hoặc mật khẩu",
        )
    };
    let Some((user_id, hash, is_active)) = row else {
        return Err(invalid());
    };
    if !is_active || !verify_password(req.password, hash).await {
        audit(
            &state.db,
            Some(&user_id),
            "auth.login_failed",
            "user",
            &user_id,
            json!({}),
        )
        .await;
        return Err(invalid());
    }
    let token = new_token();
    let max_age = state.cfg.session_hours * 3600;
    sqlx::query(
        "INSERT INTO sessions (token_hash, user_id, created_at, expires_at) VALUES (?, ?, ?, ?)",
    )
    .bind(sha256_hex(token.as_bytes()))
    .bind(&user_id)
    .bind(now())
    .bind(now_plus(max_age))
    .execute(&state.db)
    .await?;
    sqlx::query("DELETE FROM sessions WHERE expires_at < ?")
        .bind(now())
        .execute(&state.db)
        .await?;
    audit(
        &state.db,
        Some(&user_id),
        "auth.login",
        "user",
        &user_id,
        json!({}),
    )
    .await;
    let mut headers = HeaderMap::new();
    headers.insert(header::SET_COOKIE, session_cookie(&state, &token, max_age));
    Ok((headers, Json(load_user(&state, &user_id).await?)).into_response())
}

pub async fn logout(State(state): State<AppState>, user: AuthUser) -> AppResult<Response> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
        .bind(&user.session_hash)
        .execute(&state.db)
        .await?;
    let mut headers = HeaderMap::new();
    headers.insert(header::SET_COOKIE, session_cookie(&state, "", 0));
    Ok((StatusCode::NO_CONTENT, headers).into_response())
}

pub async fn me(State(state): State<AppState>, user: AuthUser) -> AppResult<Json<User>> {
    Ok(Json(load_user(&state, &user.id).await?))
}

#[derive(Deserialize)]
pub struct PasswordChange {
    current_password: String,
    new_password: String,
}

pub async fn change_password(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<PasswordChange>,
) -> AppResult<StatusCode> {
    validate_password(&req.new_password)?;
    let (hash,): (String,) = sqlx::query_as("SELECT password_hash FROM users WHERE id = ?")
        .bind(&user.id)
        .fetch_one(&state.db)
        .await?;
    if !verify_password(req.current_password, hash).await {
        return Err(AppError::validation("Mật khẩu hiện tại không đúng"));
    }
    sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
        .bind(hash_password(req.new_password).await?)
        .bind(&user.id)
        .execute(&state.db)
        .await?;
    // Other sessions are revoked; the current one stays valid.
    sqlx::query("DELETE FROM sessions WHERE user_id = ? AND token_hash <> ?")
        .bind(&user.id)
        .bind(&user.session_hash)
        .execute(&state.db)
        .await?;
    audit(
        &state.db,
        Some(&user.id),
        "user.password_changed",
        "user",
        &user.id,
        json!({}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- users (admin) ----------

pub async fn list_users(
    State(state): State<AppState>,
    _admin: Admin,
) -> AppResult<Json<Vec<User>>> {
    let users = sqlx::query_as(&format!(
        "SELECT {USER_COLUMNS} FROM users ORDER BY created_at"
    ))
    .fetch_all(&state.db)
    .await?;
    Ok(Json(users))
}

#[derive(Deserialize)]
pub struct UserCreate {
    username: String,
    password: String,
    #[serde(default)]
    display_name: String,
    role: String,
}

fn validate_role(role: &str) -> AppResult<()> {
    if !ROLES.contains(&role) {
        return Err(AppError::validation(
            "Role phải là admin, trainer hoặc member",
        ));
    }
    Ok(())
}

pub async fn insert_user(
    state: &AppState,
    username: &str,
    password: String,
    display_name: &str,
    role: &str,
) -> AppResult<String> {
    let username = username.trim();
    if username.len() < 3
        || !username
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    {
        return Err(AppError::validation(
            "Tên đăng nhập cần ít nhất 3 ký tự, chỉ gồm chữ, số và . _ -",
        ));
    }
    validate_role(role)?;
    validate_password(&password)?;
    let id = new_id();
    sqlx::query("INSERT INTO users (id, username, display_name, password_hash, role, created_at) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(&id)
        .bind(username)
        .bind(if display_name.trim().is_empty() { username } else { display_name.trim() })
        .bind(hash_password(password).await?)
        .bind(role)
        .bind(now())
        .execute(&state.db)
        .await
        .map_err(|e| match AppError::from(e) {
            e if e.status == StatusCode::CONFLICT => AppError::conflict("Tên đăng nhập đã tồn tại"),
            e => e,
        })?;
    Ok(id)
}

pub async fn create_user(
    State(state): State<AppState>,
    Admin(admin): Admin,
    Json(req): Json<UserCreate>,
) -> AppResult<(StatusCode, Json<User>)> {
    let id = insert_user(
        &state,
        &req.username,
        req.password,
        &req.display_name,
        &req.role,
    )
    .await?;
    audit(
        &state.db,
        Some(&admin.id),
        "user.created",
        "user",
        &id,
        json!({"username": req.username, "role": req.role}),
    )
    .await;
    Ok((StatusCode::CREATED, Json(load_user(&state, &id).await?)))
}

#[derive(Deserialize)]
pub struct UserUpdate {
    display_name: Option<String>,
    role: Option<String>,
    is_active: Option<bool>,
    password: Option<String>,
}

pub async fn update_user(
    State(state): State<AppState>,
    Admin(admin): Admin,
    Path(id): Path<String>,
    Json(req): Json<UserUpdate>,
) -> AppResult<Json<User>> {
    let current = load_user(&state, &id).await?;
    if let Some(role) = &req.role {
        validate_role(role)?;
    }
    // Never let the last active admin be demoted or locked out.
    let loses_admin = current.role == "admin"
        && current.is_active
        && (req.role.as_deref().is_some_and(|r| r != "admin") || req.is_active == Some(false));
    if loses_admin {
        let (admins,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM users WHERE role = 'admin' AND is_active = 1")
                .fetch_one(&state.db)
                .await?;
        if admins <= 1 {
            return Err(AppError::conflict(
                "Phải còn ít nhất một admin đang hoạt động",
            ));
        }
    }
    let password_hash = match req.password {
        Some(password) => {
            validate_password(&password)?;
            Some(hash_password(password).await?)
        }
        None => None,
    };
    sqlx::query(
        "UPDATE users SET display_name = COALESCE(?, display_name), role = COALESCE(?, role), \
         is_active = COALESCE(?, is_active), password_hash = COALESCE(?, password_hash) WHERE id = ?",
    )
    .bind(&req.display_name)
    .bind(&req.role)
    .bind(req.is_active)
    .bind(&password_hash)
    .bind(&id)
    .execute(&state.db)
    .await?;
    if req.is_active == Some(false) || password_hash.is_some() {
        sqlx::query("DELETE FROM sessions WHERE user_id = ?")
            .bind(&id)
            .execute(&state.db)
            .await?;
    }
    audit(
        &state.db,
        Some(&admin.id),
        "user.updated",
        "user",
        &id,
        json!({"role": req.role, "is_active": req.is_active, "password_reset": password_hash.is_some()}),
    )
    .await;
    Ok(Json(load_user(&state, &id).await?))
}

// ---------- projects ----------

#[derive(Serialize, FromRow)]
pub struct Project {
    id: String,
    name: String,
    description: String,
    owner_id: String,
    created_at: String,
}

#[derive(Deserialize)]
pub struct ProjectInput {
    name: Option<String>,
    description: Option<String>,
}

pub async fn list_projects(
    State(state): State<AppState>,
    user: AuthUser,
) -> AppResult<Json<Vec<Project>>> {
    let projects =
        sqlx::query_as("SELECT * FROM projects WHERE (? OR owner_id = ?) ORDER BY created_at DESC")
            .bind(user.is_admin())
            .bind(&user.id)
            .fetch_all(&state.db)
            .await?;
    Ok(Json(projects))
}

fn project_name(name: Option<&str>) -> AppResult<Option<&str>> {
    match name.map(str::trim) {
        Some("") => Err(AppError::validation("Tên project không được để trống")),
        other => Ok(other),
    }
}

pub async fn create_project(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<ProjectInput>,
) -> AppResult<(StatusCode, Json<Project>)> {
    let name = project_name(req.name.as_deref())?
        .ok_or_else(|| AppError::validation("Thiếu tên project"))?;
    let id = new_id();
    sqlx::query(
        "INSERT INTO projects (id, name, description, owner_id, created_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(name)
    .bind(req.description.as_deref().unwrap_or(""))
    .bind(&user.id)
    .bind(now())
    .execute(&state.db)
    .await?;
    let project = sqlx::query_as("SELECT * FROM projects WHERE id = ?")
        .bind(&id)
        .fetch_one(&state.db)
        .await?;
    Ok((StatusCode::CREATED, Json(project)))
}

pub async fn update_project(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(req): Json<ProjectInput>,
) -> AppResult<Json<Project>> {
    let project: Project = sqlx::query_as("SELECT * FROM projects WHERE id = ?")
        .bind(&id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::not_found("project"))?;
    if !user.can_modify(&project.owner_id) {
        return Err(AppError::forbidden("Bạn không có quyền sửa project này"));
    }
    sqlx::query("UPDATE projects SET name = COALESCE(?, name), description = COALESCE(?, description) WHERE id = ?")
        .bind(project_name(req.name.as_deref())?)
        .bind(&req.description)
        .bind(&id)
        .execute(&state.db)
        .await?;
    Ok(Json(
        sqlx::query_as("SELECT * FROM projects WHERE id = ?")
            .bind(&id)
            .fetch_one(&state.db)
            .await?,
    ))
}

// ---------- audit logs (admin) ----------

#[derive(Serialize, FromRow)]
pub struct AuditLog {
    id: i64,
    actor_id: Option<String>,
    actor_name: Option<String>,
    action: String,
    target_type: String,
    target_id: String,
    detail: DbJson<Value>,
    created_at: String,
}

#[derive(Deserialize)]
pub struct LimitQuery {
    limit: Option<i64>,
}

pub async fn list_audit_logs(
    State(state): State<AppState>,
    _admin: Admin,
    Query(q): Query<LimitQuery>,
) -> AppResult<Json<Vec<AuditLog>>> {
    let logs = sqlx::query_as(
        "SELECT a.id, a.actor_id, u.username AS actor_name, a.action, a.target_type, a.target_id, a.detail, a.created_at \
         FROM audit_logs a LEFT JOIN users u ON u.id = a.actor_id ORDER BY a.id DESC LIMIT ?",
    )
    .bind(q.limit.unwrap_or(200).clamp(1, 1000))
    .fetch_all(&state.db)
    .await?;
    Ok(Json(logs))
}
