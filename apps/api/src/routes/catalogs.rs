use axum::Json;
use serde_json::{Value, json};

use crate::auth::Trainer;

/// Stub: Task 2 replaces this with the real catalog listing.
pub async fn list_catalogs(_trainer: Trainer) -> Json<Value> {
    Json(json!([]))
}
