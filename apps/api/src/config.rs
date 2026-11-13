use std::{env, path::PathBuf};

/// Runtime configuration, read once from environment variables (see `.env.example`).
#[derive(Debug, Clone)]
pub struct Config {
    pub bind: String,
    pub database_url: String,
    /// Root for uploaded artifacts. Storage keys are always relative to this directory.
    pub data_dir: PathBuf,
    /// Built React app to serve as a fallback (optional; a reverse proxy may serve it instead).
    pub web_dist: Option<PathBuf>,
    pub cookie_secure: bool,
    pub session_hours: i64,
    pub lease_seconds: i64,
    pub heartbeat_interval: i64,
    pub poll_interval: i64,
    pub node_online_seconds: i64,
    pub max_upload_mb: usize,
    /// Largest Reference Audio / Source Audio a user may upload.
    pub max_input_audio_mb: usize,
    pub bootstrap_admin: Option<(String, String)>,
}

fn var(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn num<T: std::str::FromStr>(name: &str, default: T) -> T {
    var(name).and_then(|v| v.parse().ok()).unwrap_or(default)
}

impl Config {
    pub fn from_env() -> Self {
        let data_dir = PathBuf::from(var("DATA_DIR").unwrap_or_else(|| "data".into()));
        let database_url = var("DATABASE_URL").unwrap_or_else(|| {
            format!(
                "sqlite://{}",
                data_dir
                    .join("lush.db")
                    .to_string_lossy()
                    .replace('\\', "/")
            )
        });
        Self {
            bind: var("BIND_ADDR").unwrap_or_else(|| "127.0.0.1:8080".into()),
            database_url,
            data_dir,
            web_dist: var("WEB_DIST").map(PathBuf::from),
            cookie_secure: num("COOKIE_SECURE", false),
            session_hours: num("SESSION_HOURS", 24 * 7),
            lease_seconds: num("JOB_LEASE_SECONDS", 90),
            heartbeat_interval: num("HEARTBEAT_INTERVAL_SECONDS", 15),
            poll_interval: num("WORKER_POLL_INTERVAL_SECONDS", 3),
            node_online_seconds: num("NODE_ONLINE_SECONDS", 45),
            max_upload_mb: num("MAX_UPLOAD_MB", 512),
            max_input_audio_mb: num("MAX_INPUT_AUDIO_MB", 50),
            bootstrap_admin: var("BOOTSTRAP_ADMIN_USERNAME").zip(var("BOOTSTRAP_ADMIN_PASSWORD")),
        }
    }
}
