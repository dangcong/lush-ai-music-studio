use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use chrono::{Duration, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::error::{AppError, AppResult};

const TIME_FORMAT: &str = "%Y-%m-%dT%H:%M:%S%.3fZ";

/// Current UTC time in the fixed-width format used everywhere (sorts lexicographically).
pub fn now() -> String {
    Utc::now().format(TIME_FORMAT).to_string()
}

pub fn now_plus(seconds: i64) -> String {
    (Utc::now() + Duration::seconds(seconds))
        .format(TIME_FORMAT)
        .to_string()
}

pub fn new_id() -> String {
    Uuid::now_v7().to_string()
}

/// 244 bits of OS randomness, hex encoded. Used for session, node and lease tokens.
pub fn new_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

pub fn random_seed() -> i64 {
    let b = Uuid::new_v4().into_bytes();
    (u32::from_le_bytes([b[0], b[1], b[2], b[3]]) & 0x7fff_ffff) as i64
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

pub async fn hash_password(password: String) -> AppResult<String> {
    tokio::task::spawn_blocking(move || {
        let salt = SaltString::encode_b64(Uuid::new_v4().as_bytes()).map_err(AppError::internal)?;
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|h| h.to_string())
            .map_err(AppError::internal)
    })
    .await
    .map_err(AppError::internal)?
}

pub async fn verify_password(password: String, hash: String) -> bool {
    tokio::task::spawn_blocking(move || {
        PasswordHash::new(&hash)
            .map(|parsed| {
                Argon2::default()
                    .verify_password(password.as_bytes(), &parsed)
                    .is_ok()
            })
            .unwrap_or(false)
    })
    .await
    .unwrap_or(false)
}

pub fn validate_password(password: &str) -> AppResult<()> {
    if password.chars().count() < 8 {
        return Err(AppError::validation("Mật khẩu phải có ít nhất 8 ký tự"));
    }
    Ok(())
}

/// Shallow merge of a JSON object patch onto a base object (keys present in `patch` win, including null).
pub fn merge(mut base: Value, patch: &Value) -> AppResult<Value> {
    let (Some(target), Some(source)) = (base.as_object_mut(), patch.as_object()) else {
        return Err(AppError::bad_request("Body phải là JSON object"));
    };
    for (k, v) in source {
        target.insert(k.clone(), v.clone());
    }
    Ok(base)
}

pub async fn audit(
    db: &SqlitePool,
    actor_id: Option<&str>,
    action: &str,
    target_type: &str,
    target_id: &str,
    detail: Value,
) {
    let res = sqlx::query(
        "INSERT INTO audit_logs (actor_id, action, target_type, target_id, detail, created_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(actor_id)
    .bind(action)
    .bind(target_type)
    .bind(target_id)
    .bind(detail.to_string())
    .bind(now())
    .execute(db)
    .await;
    if let Err(err) = res {
        tracing::error!(error = %err, action, "failed to write audit log");
    }
}

pub fn empty_object() -> Value {
    json!({})
}
