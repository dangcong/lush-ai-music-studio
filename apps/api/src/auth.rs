//! Request authentication: session cookies for people, bearer tokens for render nodes.

use axum::{
    extract::FromRequestParts,
    http::{HeaderMap, StatusCode, header, request::Parts},
};

use crate::{
    AppState,
    error::AppError,
    util::{now, sha256_hex},
};

pub const SESSION_COOKIE: &str = "lush_session";

pub fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_string())
}

/// A logged-in, active user.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: String,
    pub username: String,
    pub role: String,
    pub session_hash: String,
}

impl AuthUser {
    pub fn is_admin(&self) -> bool {
        self.role == "admin"
    }
    /// Owners and admins may modify a resource.
    pub fn can_modify(&self, owner_id: &str) -> bool {
        self.is_admin() || self.id == owner_id
    }
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token =
            cookie_value(&parts.headers, SESSION_COOKIE).ok_or_else(AppError::unauthorized)?;
        let session_hash = sha256_hex(token.as_bytes());
        let row: Option<(String, String, String)> = sqlx::query_as(
            "SELECT u.id, u.username, u.role FROM sessions s JOIN users u ON u.id = s.user_id \
             WHERE s.token_hash = ? AND s.expires_at > ? AND u.is_active = 1",
        )
        .bind(&session_hash)
        .bind(now())
        .fetch_optional(&state.db)
        .await?;
        let (id, username, role) = row.ok_or_else(AppError::unauthorized)?;
        Ok(Self {
            id,
            username,
            role,
            session_hash,
        })
    }
}

/// A logged-in user with the `admin` role.
#[derive(Debug, Clone)]
pub struct Admin(pub AuthUser);

impl FromRequestParts<AppState> for Admin {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let user = AuthUser::from_request_parts(parts, state).await?;
        if !user.is_admin() {
            return Err(AppError::forbidden("Chỉ admin được thực hiện thao tác này"));
        }
        Ok(Self(user))
    }
}

/// An authenticated, enabled render node (worker protocol).
#[derive(Debug, Clone)]
pub struct NodeAuth {
    pub id: String,
    pub name: String,
}

impl FromRequestParts<AppState> for NodeAuth {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(AppError::unauthorized)?;
        let row: Option<(String, String, String)> =
            sqlx::query_as("SELECT id, name, status FROM nodes WHERE token_hash = ?")
                .bind(sha256_hex(token.as_bytes()))
                .fetch_optional(&state.db)
                .await?;
        let (id, name, status) = row.ok_or_else(AppError::unauthorized)?;
        if status != "active" {
            return Err(AppError::new(
                StatusCode::FORBIDDEN,
                "node_disabled",
                "Render node đã bị vô hiệu hóa",
            ));
        }
        Ok(Self { id, name })
    }
}
