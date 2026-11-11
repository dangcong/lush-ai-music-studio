//! Lush AI Music Studio control plane: website API, job queue and render node registry.

pub mod auth;
pub mod config;
pub mod error;
pub mod queue;
pub mod routes;
pub mod util;

use std::{str::FromStr, sync::Arc, time::Duration};

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    routing::{get, patch, post},
};
use serde_json::{Value, json};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use tower_http::{
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};

use crate::{
    config::Config,
    error::{AppError, AppResult},
    routes::{genres, jobs, registry, tracks, users, worker},
};

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub cfg: Arc<Config>,
}

/// Open the SQLite database (WAL, foreign keys on) and apply pending migrations.
pub async fn connect(database_url: &str) -> anyhow::Result<SqlitePool> {
    let options = SqliteConnectOptions::from_str(database_url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(10));
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(options)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

/// Create the first admin from BOOTSTRAP_ADMIN_* when the user table is empty.
pub async fn bootstrap_admin(state: &AppState) -> AppResult<()> {
    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
        .fetch_one(&state.db)
        .await?;
    if count > 0 {
        return Ok(());
    }
    match &state.cfg.bootstrap_admin {
        Some((username, password)) => {
            users::insert_user(state, username, password.clone(), "", "admin").await?;
            tracing::info!(%username, "bootstrap admin created");
        }
        None => tracing::warn!(
            "no users exist; set BOOTSTRAP_ADMIN_USERNAME and BOOTSTRAP_ADMIN_PASSWORD to create the first admin"
        ),
    }
    Ok(())
}

async fn health(State(state): State<AppState>) -> AppResult<Json<Value>> {
    sqlx::query("SELECT 1").execute(&state.db).await?;
    Ok(Json(
        json!({"status": "ok", "version": env!("CARGO_PKG_VERSION"), "db": "ok"}),
    ))
}

async fn api_not_found() -> AppError {
    AppError::not_found("endpoint")
}

pub fn build_router(state: AppState) -> Router {
    let api = Router::new()
        .route("/auth/login", post(users::login))
        .route("/auth/logout", post(users::logout))
        .route("/auth/me", get(users::me))
        .route("/auth/password", post(users::change_password))
        .route("/users", get(users::list_users).post(users::create_user))
        .route("/users/{id}", patch(users::update_user))
        .route(
            "/projects",
            get(users::list_projects).post(users::create_project),
        )
        .route("/projects/{id}", patch(users::update_project))
        .route("/audit-logs", get(users::list_audit_logs))
        .route(
            "/genres",
            get(genres::list_genres).post(genres::create_genre),
        )
        .route("/genres/{id}", patch(genres::update_genre))
        .route(
            "/presets",
            get(genres::list_presets).post(genres::create_preset),
        )
        .route("/presets/{id}", patch(genres::update_preset))
        .route("/models", get(registry::list_models))
        .route("/models/{id}", patch(registry::update_model))
        .route("/loras", get(registry::list_loras))
        .route(
            "/nodes",
            get(registry::list_nodes).post(registry::create_node),
        )
        .route("/nodes/{id}", patch(registry::update_node))
        .route(
            "/nodes/{id}/rotate-token",
            post(registry::rotate_node_token),
        )
        .route("/generations", post(jobs::create_generation))
        .route("/jobs", get(jobs::list_jobs))
        .route("/jobs/{id}", get(jobs::get_job))
        .route("/jobs/{id}/cancel", post(jobs::cancel_job))
        .route("/jobs/{id}/retry", post(jobs::retry_job))
        .route("/queue/summary", get(jobs::queue_summary))
        .route("/tracks", get(tracks::list_tracks))
        .route(
            "/tracks/{id}",
            get(tracks::get_track)
                .patch(tracks::update_track)
                .delete(tracks::delete_track),
        )
        .route("/tracks/{id}/archive", get(tracks::track_archive))
        .route("/artifacts/{id}/content", get(tracks::artifact_content))
        .fallback(api_not_found);

    let upload_limit = state.cfg.max_upload_mb * 1024 * 1024;
    let worker_api = Router::new()
        .route("/register", post(worker::register))
        .route("/heartbeat", post(worker::heartbeat))
        .route("/jobs/claim", post(worker::claim))
        .route("/jobs/{id}/progress", post(worker::progress))
        .route(
            "/jobs/{id}/artifacts",
            post(worker::upload_artifact).layer(DefaultBodyLimit::max(upload_limit)),
        )
        .route("/jobs/{id}/complete", post(worker::complete))
        .route("/jobs/{id}/fail", post(worker::fail))
        .route("/jobs/{id}/release", post(worker::release))
        .fallback(api_not_found);

    let mut app = Router::new()
        .route("/api/health", get(health))
        .nest("/api/v1", api)
        .nest("/api/worker/v1", worker_api)
        .route("/api/{*rest}", get(api_not_found).post(api_not_found));

    // Optionally serve the built React app (single-page app: unknown paths fall back to index.html).
    if let Some(dist) = &state.cfg.web_dist {
        app = app.fallback_service(
            ServeDir::new(dist).fallback(ServeFile::new(dist.join("index.html"))),
        );
    }
    app.layer(TraceLayer::new_for_http()).with_state(state)
}
