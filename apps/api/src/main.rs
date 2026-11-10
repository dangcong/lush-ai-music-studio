use std::sync::Arc;

use lush_api::{
    AppState, bootstrap_admin, build_router, config::Config, connect, queue::spawn_reaper,
};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tower_http=warn,sqlx=warn".into()),
        )
        .init();

    let cfg = Config::from_env();
    std::fs::create_dir_all(&cfg.data_dir)?;
    let db = connect(&cfg.database_url).await?;
    let state = AppState {
        db,
        cfg: Arc::new(cfg),
    };
    bootstrap_admin(&state)
        .await
        .map_err(|e| anyhow::anyhow!(e.message))?;
    spawn_reaper(state.clone());

    let listener = tokio::net::TcpListener::bind(&state.cfg.bind).await?;
    tracing::info!(addr = %state.cfg.bind, "lush-api listening");
    axum::serve(listener, build_router(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
