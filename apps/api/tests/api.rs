//! End-to-end tests against the real router and a temporary SQLite database.
//! The "worker" here is the test itself speaking the worker protocol over HTTP.

use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use http_body_util::BodyExt;
use lush_api::{
    AppState, build_router, config::Config, connect, queue::recover_jobs,
    routes::users::insert_user, util::sha256_hex,
};
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;

struct TestApp {
    router: Router,
    state: AppState,
    _dir: TempDir,
}

struct Reply {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    bytes: Vec<u8>,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.bytes)
            .unwrap_or_else(|_| panic!("not JSON: {}", String::from_utf8_lossy(&self.bytes)))
    }
    fn code(&self) -> String {
        self.json()["error"]["code"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }
}

impl TestApp {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = Config::from_env();
        cfg.data_dir = dir.path().to_path_buf();
        cfg.database_url = format!(
            "sqlite://{}",
            dir.path()
                .join("test.db")
                .to_string_lossy()
                .replace('\\', "/")
        );
        cfg.web_dist = None;
        let db = connect(&cfg.database_url).await.unwrap();
        let state = AppState {
            db,
            cfg: Arc::new(cfg),
        };
        insert_user(&state, "admin", "admin-password".into(), "Admin", "admin")
            .await
            .unwrap();
        insert_user(&state, "alice", "alice-password".into(), "Alice", "member")
            .await
            .unwrap();
        insert_user(&state, "bob", "bob-password".into(), "Bob", "member")
            .await
            .unwrap();
        Self {
            router: build_router(state.clone()),
            state,
            _dir: dir,
        }
    }

    async fn send(&self, request: Request<Body>) -> Reply {
        let response = self.router.clone().oneshot(request).await.unwrap();
        let (parts, body) = response.into_parts();
        Reply {
            status: parts.status,
            headers: parts.headers,
            bytes: body.collect().await.unwrap().to_bytes().to_vec(),
        }
    }

    /// JSON request authenticated with either a session cookie or a node bearer token.
    async fn call(&self, method: &str, path: &str, auth: &str, body: Option<Value>) -> Reply {
        let mut request = Request::builder().method(method).uri(path);
        if auth.starts_with("lush_session=") {
            request = request.header(header::COOKIE, auth);
        } else if !auth.is_empty() {
            request = request.header(header::AUTHORIZATION, format!("Bearer {auth}"));
        }
        let body = match body {
            Some(value) => {
                request = request.header(header::CONTENT_TYPE, "application/json");
                Body::from(value.to_string())
            }
            None => Body::empty(),
        };
        self.send(request.body(body).unwrap()).await
    }

    async fn login(&self, username: &str) -> String {
        let reply = self
            .call(
                "POST",
                "/api/v1/auth/login",
                "",
                Some(json!({"username": username, "password": format!("{username}-password")})),
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK, "login failed");
        let cookie = reply
            .headers
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap();
        cookie.split(';').next().unwrap().to_string()
    }

    /// Create a node as admin and register it with the mock model. Returns (node_id, token).
    async fn node(&self, admin: &str, name: &str, models: Value) -> (String, String) {
        let created = self
            .call("POST", "/api/v1/nodes", admin, Some(json!({"name": name})))
            .await;
        assert_eq!(created.status, StatusCode::CREATED);
        let token = created.json()["token"].as_str().unwrap().to_string();
        let reply = self.register(&token, models, true).await;
        assert_eq!(
            reply.status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&reply.bytes)
        );
        (reply.json()["node_id"].as_str().unwrap().to_string(), token)
    }

    async fn register(&self, token: &str, models: Value, fresh_start: bool) -> Reply {
        let body = json!({
            "protocol_version": 1, "worker_version": "test", "fresh_start": fresh_start,
            "job_types": ["generate"], "engines": [{"name": "mock", "version": "1", "capabilities": ["generate"]}],
            "models": models, "loras": [], "gpu": null,
        });
        self.call("POST", "/api/worker/v1/register", token, Some(body))
            .await
    }

    async fn model_id(&self, cookie: &str, key: &str) -> String {
        let models = self
            .call("GET", "/api/v1/models", cookie, None)
            .await
            .json();
        models
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["key"] == key)
            .expect("model registered")["id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    async fn generate(&self, cookie: &str, body: Value) -> Reply {
        self.call("POST", "/api/v1/generations", cookie, Some(body))
            .await
    }

    async fn claim(&self, token: &str) -> Reply {
        self.call("POST", "/api/worker/v1/jobs/claim", token, Some(json!({})))
            .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn upload(
        &self,
        token: &str,
        job_id: &str,
        lease: &str,
        kind: &str,
        file_name: &str,
        data: &[u8],
        sha: &str,
    ) -> Reply {
        let boundary = "lushtestboundary";
        let mut body = Vec::new();
        for (name, value) in [("lease_token", lease), ("kind", kind), ("sha256", sha)] {
            body.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").bytes());
        }
        body.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{file_name}\"\r\nContent-Type: application/octet-stream\r\n\r\n").bytes());
        body.extend_from_slice(data);
        body.extend(format!("\r\n--{boundary}--\r\n").bytes());
        let request = Request::builder()
            .method("POST")
            .uri(format!("/api/worker/v1/jobs/{job_id}/artifacts"))
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body))
            .unwrap();
        self.send(request).await
    }

    async fn job(&self, cookie: &str, id: &str) -> Value {
        self.call("GET", &format!("/api/v1/jobs/{id}"), cookie, None)
            .await
            .json()
    }

    /// Force the lease of a job into the past and run the reaper once.
    async fn expire_lease(&self, job_id: &str) {
        sqlx::query("UPDATE jobs SET lease_expires_at = '2000-01-01T00:00:00.000Z' WHERE id = ?")
            .bind(job_id)
            .execute(&self.state.db)
            .await
            .unwrap();
        recover_jobs(&self.state.db, None, "lease expired")
            .await
            .unwrap();
    }
}

fn mock_models() -> Value {
    json!([{"engine": "mock", "key": "mock-v1", "name": "Mock Engine v1", "capabilities": ["generate"]}])
}

fn request_body(model_id: &str) -> Value {
    json!({"prompt": "Warm jazz piano", "model_id": model_id, "duration": 30})
}

const WAV: &[u8] = b"RIFF....WAVEfmt fake audio bytes for tests";

// ---------- auth & permissions ----------

#[tokio::test]
async fn auth_and_permissions() {
    let app = TestApp::new().await;
    assert_eq!(
        app.call("GET", "/api/health", "", None).await.json()["status"],
        "ok"
    );
    assert_eq!(
        app.call("GET", "/api/v1/auth/me", "", None).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.call("GET", "/api/v1/tracks", "lush_session=bogus", None)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    let bad = app
        .call(
            "POST",
            "/api/v1/auth/login",
            "",
            Some(json!({"username": "alice", "password": "wrong"})),
        )
        .await;
    assert_eq!(bad.status, StatusCode::UNAUTHORIZED);

    let alice = app.login("alice").await;
    let me = app
        .call("GET", "/api/v1/auth/me", &alice, None)
        .await
        .json();
    assert_eq!(me["username"], "alice");
    assert_eq!(me["role"], "member");
    assert!(me.get("password_hash").is_none());

    // Members cannot reach admin endpoints.
    for (method, path, body) in [
        ("GET", "/api/v1/users", None),
        ("GET", "/api/v1/nodes", None),
        ("GET", "/api/v1/audit-logs", None),
        ("POST", "/api/v1/nodes", Some(json!({"name": "evil-node"}))),
        (
            "POST",
            "/api/v1/genres",
            Some(json!({"name": "X", "slug": "x"})),
        ),
    ] {
        assert_eq!(
            app.call(method, path, &alice, body).await.status,
            StatusCode::FORBIDDEN,
            "{method} {path}"
        );
    }

    // Logout invalidates the session.
    assert_eq!(
        app.call("POST", "/api/v1/auth/logout", &alice, None)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.call("GET", "/api/v1/auth/me", &alice, None)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.call("GET", "/api/v1/nope", "", None).await.code(),
        "not_found"
    );
}

#[tokio::test]
async fn user_management() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let created = app
        .call(
            "POST",
            "/api/v1/users",
            &admin,
            Some(json!({"username": "carol", "password": "carol-password", "role": "trainer"})),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let carol_id = created.json()["id"].as_str().unwrap().to_string();
    let dup = app
        .call(
            "POST",
            "/api/v1/users",
            &admin,
            Some(json!({"username": "CAROL", "password": "carol-password", "role": "member"})),
        )
        .await;
    assert_eq!(dup.status, StatusCode::CONFLICT);
    let weak = app
        .call(
            "POST",
            "/api/v1/users",
            &admin,
            Some(json!({"username": "dave", "password": "short", "role": "member"})),
        )
        .await;
    assert_eq!(weak.status, StatusCode::UNPROCESSABLE_ENTITY);

    // Deactivating a user kills their sessions.
    let carol = app.login("carol").await;
    let off = app
        .call(
            "PATCH",
            &format!("/api/v1/users/{carol_id}"),
            &admin,
            Some(json!({"is_active": false})),
        )
        .await;
    assert_eq!(off.json()["is_active"], false);
    assert_eq!(
        app.call("GET", "/api/v1/auth/me", &carol, None)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );

    // The last admin cannot be demoted.
    let admin_id = app
        .call("GET", "/api/v1/auth/me", &admin, None)
        .await
        .json()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let demote = app
        .call(
            "PATCH",
            &format!("/api/v1/users/{admin_id}"),
            &admin,
            Some(json!({"role": "member"})),
        )
        .await;
    assert_eq!(demote.status, StatusCode::CONFLICT);

    // Password change keeps the session and the new password works.
    let change = app
        .call(
            "POST",
            "/api/v1/auth/password",
            &admin,
            Some(
                json!({"current_password": "admin-password", "new_password": "new-admin-password"}),
            ),
        )
        .await;
    assert_eq!(change.status, StatusCode::NO_CONTENT);
    let relogin = app
        .call(
            "POST",
            "/api/v1/auth/login",
            "",
            Some(json!({"username": "admin", "password": "new-admin-password"})),
        )
        .await;
    assert_eq!(relogin.status, StatusCode::OK);
    let logs = app
        .call("GET", "/api/v1/audit-logs", &admin, None)
        .await
        .json();
    assert!(
        logs.as_array()
            .unwrap()
            .iter()
            .any(|l| l["action"] == "user.created")
    );
}

// ---------- genre registry ----------

#[tokio::test]
async fn genre_registry_is_data_driven() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let alice = app.login("alice").await;

    // Seeded by migration: 4 genres with their subgenres.
    let genres = app.call("GET", "/api/v1/genres", &alice, None).await.json();
    let genres = genres.as_array().unwrap();
    let roots: Vec<&Value> = genres.iter().filter(|g| g["parent_id"].is_null()).collect();
    assert_eq!(roots.len(), 4);
    assert_eq!(genres.len(), 23);
    let lofi = roots.iter().find(|g| g["slug"] == "lo-fi").unwrap();
    assert!(lofi["instruments"].as_array().unwrap().len() > 1);

    // A new genre and subgenre appear with no code change.
    let synth = app.call("POST", "/api/v1/genres", &admin, Some(json!({"name": "Synthwave", "slug": "synthwave", "prompt_template": "synthwave, retro"}))).await;
    assert_eq!(synth.status, StatusCode::CREATED);
    let synth_id = synth.json()["id"].as_str().unwrap().to_string();
    let sub = app
        .call(
            "POST",
            "/api/v1/genres",
            &admin,
            Some(json!({"name": "Outrun", "slug": "outrun", "parent_id": synth_id})),
        )
        .await;
    assert_eq!(sub.status, StatusCode::CREATED);
    let sub_id = sub.json()["id"].as_str().unwrap().to_string();

    // Validation: duplicate slug, three levels, bad slug.
    assert_eq!(
        app.call(
            "POST",
            "/api/v1/genres",
            &admin,
            Some(json!({"name": "Dup", "slug": "synthwave"}))
        )
        .await
        .status,
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.call(
            "POST",
            "/api/v1/genres",
            &admin,
            Some(json!({"name": "Deep", "slug": "deep", "parent_id": sub_id}))
        )
        .await
        .status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        app.call(
            "POST",
            "/api/v1/genres",
            &admin,
            Some(json!({"name": "Bad", "slug": "Bad Slug"}))
        )
        .await
        .status,
        StatusCode::UNPROCESSABLE_ENTITY
    );

    // Deactivated genres disappear for members but stay visible to admins on request.
    let off = app
        .call(
            "PATCH",
            &format!("/api/v1/genres/{synth_id}"),
            &admin,
            Some(json!({"is_active": false, "bpm_min": 90})),
        )
        .await;
    assert_eq!(off.json()["is_active"], false);
    assert_eq!(off.json()["bpm_min"], 90);
    assert_eq!(off.json()["prompt_template"], "synthwave, retro");
    let visible = app
        .call("GET", "/api/v1/genres?include_inactive=true", &alice, None)
        .await
        .json();
    assert!(
        !visible
            .as_array()
            .unwrap()
            .iter()
            .any(|g| g["slug"] == "synthwave")
    );
    let all = app
        .call("GET", "/api/v1/genres?include_inactive=true", &admin, None)
        .await
        .json();
    assert!(
        all.as_array()
            .unwrap()
            .iter()
            .any(|g| g["slug"] == "synthwave")
    );

    // Presets: trained presets need a LoRA; nullable fields can be cleared.
    let lofi_id = lofi["id"].as_str().unwrap();
    let bad = app
        .call(
            "POST",
            "/api/v1/presets",
            &admin,
            Some(json!({"genre_id": lofi_id, "name": "Trained", "kind": "trained"})),
        )
        .await;
    assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY);
    let preset = app
        .call(
            "POST",
            "/api/v1/presets",
            &admin,
            Some(json!({"genre_id": lofi_id, "name": "Warm Lo-fi", "bpm": 80, "moods": ["warm"]})),
        )
        .await;
    assert_eq!(preset.status, StatusCode::CREATED);
    let preset_id = preset.json()["id"].as_str().unwrap().to_string();
    let cleared = app
        .call(
            "PATCH",
            &format!("/api/v1/presets/{preset_id}"),
            &admin,
            Some(json!({"bpm": null})),
        )
        .await
        .json();
    assert!(cleared["bpm"].is_null());
    assert_eq!(cleared["moods"], json!(["warm"]));
    let listed = app
        .call(
            "GET",
            &format!("/api/v1/presets?genre_id={lofi_id}"),
            &alice,
            None,
        )
        .await
        .json();
    assert_eq!(listed.as_array().unwrap().len(), 1);
}

// ---------- the full generation flow ----------

#[tokio::test]
async fn generation_flow_end_to_end() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let alice = app.login("alice").await;
    let (node_id, token) = app.node(&admin, "render-node-01", mock_models()).await;
    let model_id = app.model_id(&alice, "mock-v1").await;

    let genres = app.call("GET", "/api/v1/genres", &alice, None).await.json();
    let jazz = genres
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["slug"] == "jazz")
        .unwrap()
        .clone();
    let sub = genres
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["slug"] == "piano-jazz")
        .unwrap()
        .clone();

    let created = app.generate(&alice, json!({
        "prompt": "Rainy night trio", "title": "Rainy Night", "genre_id": jazz["id"], "subgenre_id": sub["id"],
        "model_id": model_id, "duration": 60, "bpm": 90, "key": "A minor", "instruments": ["piano"], "moods": ["warm"],
        "params": {"inference_steps": 8, "guidance_scale": null}, "idempotency_key": "click-1",
    })).await;
    assert_eq!(
        created.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&created.bytes)
    );
    let job = created.json();
    let job_id = job["id"].as_str().unwrap().to_string();
    assert_eq!(job["status"], "queued");
    assert_eq!(job["owner_name"], "alice");

    // Double submit returns the same job.
    let again = app.generate(&alice, json!({"prompt": "Rainy night trio", "model_id": model_id, "duration": 60, "idempotency_key": "click-1"})).await;
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(again.json()["id"], job_id);
    let summary = app
        .call("GET", "/api/v1/queue/summary", &alice, None)
        .await
        .json();
    assert_eq!(summary["queued"], 1);
    assert_eq!(summary["nodes_online"], 1);

    // Worker claims: payload follows the protocol, subgenre template wins over genre template.
    let claimed = app.claim(&token).await;
    assert_eq!(claimed.status, StatusCode::OK);
    let claim = claimed.json();
    let lease = claim["lease_token"].as_str().unwrap().to_string();
    let payload = &claim["payload"];
    assert_eq!(claim["id"], job_id);
    assert_eq!(claim["attempt"], 1);
    assert_eq!(
        payload["prompt"],
        "Rainy night trio, piano jazz trio, expressive piano, walking bass, piano, warm, instrumental, no vocals"
    );
    assert_eq!(payload["duration"], 60);
    assert_eq!(payload["bpm"], 90);
    assert_eq!(
        payload["model"],
        json!({"id": model_id, "engine": "mock", "key": "mock-v1"})
    );
    assert!(payload["lora"].is_null());
    assert_eq!(payload["params"], json!({"inference_steps": 8}));
    assert!(payload["seed"].as_i64().unwrap() >= 0);
    assert_eq!(app.job(&alice, &job_id).await["status"], "preparing");

    // One GPU job per node: nothing else is handed out while this one is held.
    app.generate(&alice, request_body(&model_id)).await;
    assert_eq!(app.claim(&token).await.status, StatusCode::NO_CONTENT);

    let progress = app.call("POST", &format!("/api/worker/v1/jobs/{job_id}/progress"), &token, Some(json!({"lease_token": lease, "progress": 0.5, "stage": "running", "message": "step 4/8", "metrics": {}}))).await;
    assert_eq!(progress.json()["cancel_requested"], false);
    let running = app.job(&alice, &job_id).await;
    assert_eq!(running["status"], "running");
    assert_eq!(running["progress"], 0.5);
    assert_eq!(running["node_name"], "render-node-01");
    assert!(running["log"].as_str().unwrap().contains("step 4/8"));

    // Completing without the master audio is rejected.
    let early = app
        .call(
            "POST",
            &format!("/api/worker/v1/jobs/{job_id}/complete"),
            &token,
            Some(json!({"lease_token": lease, "result": {}})),
        )
        .await;
    assert_eq!(early.code(), "missing_artifact");

    // Upload: wrong checksum and wrong type are rejected; correct upload is idempotent.
    let sha = sha256_hex(WAV);
    assert_eq!(
        app.upload(
            &token,
            &job_id,
            &lease,
            "audio_master",
            "master.wav",
            WAV,
            &"0".repeat(64)
        )
        .await
        .code(),
        "checksum_mismatch"
    );
    assert_eq!(
        app.upload(
            &token,
            &job_id,
            &lease,
            "audio_master",
            "../../evil.exe",
            WAV,
            &sha
        )
        .await
        .status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        app.upload(
            &token,
            &job_id,
            "wrong-lease",
            "audio_master",
            "master.wav",
            WAV,
            &sha
        )
        .await
        .code(),
        "lease_lost"
    );
    let first = app
        .upload(
            &token,
            &job_id,
            &lease,
            "audio_master",
            "master.wav",
            WAV,
            &sha,
        )
        .await;
    assert_eq!(
        first.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&first.bytes)
    );
    let second = app
        .upload(
            &token,
            &job_id,
            &lease,
            "audio_master",
            "master.wav",
            WAV,
            &sha,
        )
        .await;
    assert_eq!(first.json()["artifact_id"], second.json()["artifact_id"]);
    let mp3 = b"ID3 fake mp3";
    assert_eq!(
        app.upload(
            &token,
            &job_id,
            &lease,
            "audio_preview",
            "preview.mp3",
            mp3,
            &sha256_hex(mp3)
        )
        .await
        .status,
        StatusCode::OK
    );

    let complete_body = json!({"lease_token": lease, "result": {"seed": 4242, "duration": 60.5, "sample_rate": 44100, "engine": "mock", "model_key": "mock-v1"}});
    let done = app
        .call(
            "POST",
            &format!("/api/worker/v1/jobs/{job_id}/complete"),
            &token,
            Some(complete_body.clone()),
        )
        .await;
    assert_eq!(
        done.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&done.bytes)
    );
    assert_eq!(
        app.call(
            "POST",
            &format!("/api/worker/v1/jobs/{job_id}/complete"),
            &token,
            Some(complete_body)
        )
        .await
        .status,
        StatusCode::OK
    );

    let finished = app.job(&alice, &job_id).await;
    assert_eq!(finished["status"], "completed");
    assert_eq!(finished["progress"], 1.0);
    let track_id = finished["track_id"].as_str().unwrap().to_string();

    // The library has exactly one track with accurate metadata.
    let tracks = app
        .call("GET", "/api/v1/tracks?q=rainy", &alice, None)
        .await
        .json();
    assert_eq!(tracks.as_array().unwrap().len(), 1);
    let track = &tracks[0];
    assert_eq!(track["id"], track_id);
    assert_eq!(track["title"], "Rainy Night");
    assert_eq!(track["prompt"], "Rainy night trio");
    assert_eq!(track["genre_name"], "Jazz");
    assert_eq!(track["model_name"], "Mock Engine v1");
    assert_eq!(track["seed"], 4242);
    assert_eq!(track["bpm"], 90);
    assert_eq!(track["key"], "A minor");
    assert_eq!(track["duration"], 60.5);
    assert_eq!(track["params"]["sample_rate"], 44100);
    let artifacts = track["artifacts"].as_array().unwrap();
    assert_eq!(artifacts.len(), 2);
    let master = artifacts
        .iter()
        .find(|a| a["kind"] == "audio_master")
        .unwrap();
    assert_eq!(master["sha256"], sha);
    assert!(master.get("storage_key").is_none());

    // Download, range requests, and access control.
    let url = format!(
        "/api/v1/artifacts/{}/content",
        master["id"].as_str().unwrap()
    );
    let file = app.call("GET", &url, &alice, None).await;
    assert_eq!(file.status, StatusCode::OK);
    assert_eq!(file.bytes, WAV);
    assert_eq!(file.headers[header::CONTENT_TYPE], "audio/wav");
    let ranged = app
        .send(
            Request::builder()
                .uri(&url)
                .header(header::COOKIE, &alice)
                .header(header::RANGE, "bytes=0-3")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(ranged.status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(ranged.bytes, b"RIFF");
    let download = app
        .call("GET", &format!("{url}?download=true"), &alice, None)
        .await;
    assert!(
        download.headers[header::CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .starts_with("attachment")
    );
    assert_eq!(
        app.call("GET", &url, "", None).await.status,
        StatusCode::UNAUTHORIZED
    );

    // Bob can listen (shared library) but cannot edit or delete Alice's track.
    let bob = app.login("bob").await;
    assert_eq!(
        app.call("GET", &format!("/api/v1/tracks/{track_id}"), &bob, None)
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        app.call(
            "PATCH",
            &format!("/api/v1/tracks/{track_id}"),
            &bob,
            Some(json!({"title": "Mine"}))
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.call("DELETE", &format!("/api/v1/tracks/{track_id}"), &bob, None)
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.call("GET", &format!("/api/v1/jobs/{job_id}"), &bob, None)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    let updated = app
        .call(
            "PATCH",
            &format!("/api/v1/tracks/{track_id}"),
            &alice,
            Some(json!({"favorite": true, "rating": 4})),
        )
        .await
        .json();
    assert_eq!(updated["favorite"], true);
    assert_eq!(updated["rating"], 4);
    assert_eq!(
        app.call("GET", "/api/v1/tracks?favorite=true", &bob, None)
            .await
            .json()
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // The node is free again and takes the next job; node dashboard reflects reality.
    let next = app.claim(&token).await;
    assert_eq!(next.status, StatusCode::OK);
    let nodes = app.call("GET", "/api/v1/nodes", &admin, None).await.json();
    assert_eq!(nodes[0]["id"], node_id);
    assert_eq!(nodes[0]["online"], true);
    assert_eq!(nodes[0]["current_job_id"], next.json()["id"]);
    assert_eq!(nodes[0]["models"][0]["key"], "mock-v1");

    // Soft delete hides the track and its files.
    assert_eq!(
        app.call(
            "DELETE",
            &format!("/api/v1/tracks/{track_id}"),
            &alice,
            None
        )
        .await
        .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.call("GET", &url, &alice, None).await.status,
        StatusCode::NOT_FOUND
    );
    assert!(
        app.call("GET", "/api/v1/tracks", &alice, None)
            .await
            .json()
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn generation_validation() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let alice = app.login("alice").await;
    app.node(&admin, "render-node-01", mock_models()).await;
    let model_id = app.model_id(&alice, "mock-v1").await;

    for body in [
        json!({"prompt": "", "model_id": model_id, "duration": 30}),
        json!({"prompt": "x", "model_id": model_id, "duration": 5}),
        json!({"prompt": "x", "model_id": model_id, "duration": 600}),
        json!({"prompt": "x", "duration": 30}),
        json!({"prompt": "x", "model_id": "missing", "duration": 30}),
        json!({"prompt": "x", "model_id": model_id, "duration": 30, "bpm": 999}),
        json!({"prompt": "x", "model_id": model_id, "duration": 30, "seed": -1}),
        json!({"prompt": "x", "model_id": model_id, "duration": 30, "lora_id": "missing"}),
    ] {
        let reply = app.generate(&alice, body.clone()).await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    }

    // A disabled model is refused.
    app.call(
        "PATCH",
        &format!("/api/v1/models/{model_id}"),
        &admin,
        Some(json!({"status": "disabled"})),
    )
    .await;
    assert_eq!(
        app.generate(&alice, request_body(&model_id)).await.status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

// ---------- cancellation, retries, recovery ----------

#[tokio::test]
async fn cancel_queued_and_running_jobs() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let alice = app.login("alice").await;
    let bob = app.login("bob").await;
    let (_, token) = app.node(&admin, "render-node-01", mock_models()).await;
    let model_id = app.model_id(&alice, "mock-v1").await;

    // Queued: cancelled immediately, never handed to a worker. Other users cannot cancel it.
    let queued = app.generate(&alice, request_body(&model_id)).await.json()["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        app.call("POST", &format!("/api/v1/jobs/{queued}/cancel"), &bob, None)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.call(
            "POST",
            &format!("/api/v1/jobs/{queued}/cancel"),
            &alice,
            None
        )
        .await
        .json()["status"],
        "cancelled"
    );
    assert_eq!(app.claim(&token).await.status, StatusCode::NO_CONTENT);
    assert_eq!(
        app.call(
            "POST",
            &format!("/api/v1/jobs/{queued}/cancel"),
            &alice,
            None
        )
        .await
        .status,
        StatusCode::CONFLICT
    );

    // Running: flagged, surfaced to the worker via progress and heartbeat, confirmed by fail(cancelled).
    let running = app.generate(&alice, request_body(&model_id)).await.json()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let lease = app.claim(&token).await.json()["lease_token"]
        .as_str()
        .unwrap()
        .to_string();
    let flagged = app
        .call(
            "POST",
            &format!("/api/v1/jobs/{running}/cancel"),
            &alice,
            None,
        )
        .await
        .json();
    assert_eq!(flagged["status"], "preparing");
    assert_eq!(flagged["cancel_requested"], true);
    let beat = app
        .call(
            "POST",
            "/api/worker/v1/heartbeat",
            &token,
            Some(json!({"current_job_id": running, "gpu": null})),
        )
        .await
        .json();
    assert_eq!(beat["cancel_job_ids"], json!([running]));
    let progress = app
        .call(
            "POST",
            &format!("/api/worker/v1/jobs/{running}/progress"),
            &token,
            Some(json!({"lease_token": lease, "progress": 0.1, "stage": "running"})),
        )
        .await;
    assert_eq!(progress.json()["cancel_requested"], true);
    let failed = app
        .call(
            "POST",
            &format!("/api/worker/v1/jobs/{running}/fail"),
            &token,
            Some(json!({"lease_token": lease, "error": "cancelled", "cancelled": true})),
        )
        .await;
    assert_eq!(failed.json()["status"], "cancelled");
    assert_eq!(app.job(&alice, &running).await["status"], "cancelled");

    // Retry creates a fresh job with the same request and seed.
    let retried = app
        .call(
            "POST",
            &format!("/api/v1/jobs/{running}/retry"),
            &alice,
            None,
        )
        .await;
    assert_eq!(retried.status, StatusCode::CREATED);
    assert_ne!(retried.json()["id"], running);
    assert_eq!(retried.json()["status"], "queued");
    assert_eq!(
        app.call(
            "POST",
            &format!(
                "/api/v1/jobs/{}/retry",
                retried.json()["id"].as_str().unwrap()
            ),
            &alice,
            None
        )
        .await
        .status,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn worker_failures_and_retries() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let alice = app.login("alice").await;
    let (_, token) = app.node(&admin, "render-node-01", mock_models()).await;
    let model_id = app.model_id(&alice, "mock-v1").await;
    let job_id = app.generate(&alice, request_body(&model_id)).await.json()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let fail = |lease: String, retryable: bool| {
        let (app, token, job_id) = (&app, &token, &job_id);
        async move {
            app.call("POST", &format!("/api/worker/v1/jobs/{job_id}/fail"), token, Some(json!({"lease_token": lease, "error": "engine unavailable", "retryable": retryable}))).await
        }
    };

    // Retryable failures requeue until attempts run out (max 3).
    for attempt in 1..=3 {
        let claim = app.claim(&token).await.json();
        assert_eq!(claim["attempt"], attempt);
        let status = fail(claim["lease_token"].as_str().unwrap().to_string(), true)
            .await
            .json()["status"]
            .clone();
        assert_eq!(status, if attempt < 3 { "queued" } else { "failed" });
    }
    let failed = app.job(&alice, &job_id).await;
    assert_eq!(failed["error"], "engine unavailable");
    assert!(failed["finished_at"].is_string());

    // A non-retryable failure fails at once; a stale lease is rejected.
    let second = app.generate(&alice, request_body(&model_id)).await.json()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let lease = app.claim(&token).await.json()["lease_token"]
        .as_str()
        .unwrap()
        .to_string();
    let stale = app
        .call(
            "POST",
            &format!("/api/worker/v1/jobs/{second}/fail"),
            &token,
            Some(json!({"lease_token": "stale", "error": "x"})),
        )
        .await;
    assert_eq!(stale.code(), "lease_lost");
    let hard = app
        .call(
            "POST",
            &format!("/api/worker/v1/jobs/{second}/fail"),
            &token,
            Some(json!({"lease_token": lease, "error": "bad params", "retryable": false})),
        )
        .await;
    assert_eq!(hard.json()["status"], "failed");

    // Release hands the job back without spending an attempt.
    let third = app.generate(&alice, request_body(&model_id)).await.json()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let lease = app.claim(&token).await.json()["lease_token"]
        .as_str()
        .unwrap()
        .to_string();
    let released = app
        .call(
            "POST",
            &format!("/api/worker/v1/jobs/{third}/release"),
            &token,
            Some(json!({"lease_token": lease})),
        )
        .await;
    assert_eq!(released.json()["status"], "queued");
    assert_eq!(app.claim(&token).await.json()["attempt"], 1);
}

#[tokio::test]
async fn node_offline_recovery() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let alice = app.login("alice").await;
    let (_, token) = app.node(&admin, "render-node-01", mock_models()).await;
    let model_id = app.model_id(&alice, "mock-v1").await;
    let job_id = app.generate(&alice, request_body(&model_id)).await.json()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Worker crashes mid-job: the lease expires and the job goes back to the queue.
    let old_lease = app.claim(&token).await.json()["lease_token"]
        .as_str()
        .unwrap()
        .to_string();
    app.expire_lease(&job_id).await;
    let requeued = app.job(&alice, &job_id).await;
    assert_eq!(requeued["status"], "queued");
    assert!(requeued["node_id"].is_null());
    assert!(requeued["log"].as_str().unwrap().contains("lease expired"));

    // The dead worker's late reports are rejected; the website keeps working meanwhile.
    let late = app
        .call(
            "POST",
            &format!("/api/worker/v1/jobs/{job_id}/progress"),
            &token,
            Some(json!({"lease_token": old_lease, "progress": 0.9})),
        )
        .await;
    assert_eq!(late.code(), "lease_lost");
    assert_eq!(
        app.call("GET", "/api/v1/tracks", &alice, None).await.status,
        StatusCode::OK
    );

    // Worker restarts (fresh_start) while holding a job: the job is recovered straight away.
    assert_eq!(app.claim(&token).await.json()["attempt"], 2);
    assert_eq!(
        app.register(&token, mock_models(), false).await.status,
        StatusCode::OK
    );
    assert_eq!(
        app.job(&alice, &job_id).await["status"],
        "preparing",
        "periodic re-register must not touch jobs"
    );
    app.register(&token, mock_models(), true).await;
    assert_eq!(app.job(&alice, &job_id).await["status"], "queued");

    // Attempts are bounded: the third lost lease fails the job for good.
    assert_eq!(app.claim(&token).await.json()["attempt"], 3);
    app.expire_lease(&job_id).await;
    let dead = app.job(&alice, &job_id).await;
    assert_eq!(dead["status"], "failed");
    assert_eq!(dead["error"], "lease expired");

    // A heartbeat with the current job keeps the lease alive.
    let kept = app.generate(&alice, request_body(&model_id)).await.json()["id"]
        .as_str()
        .unwrap()
        .to_string();
    app.claim(&token).await;
    sqlx::query("UPDATE jobs SET lease_expires_at = '2000-01-01T00:00:00.000Z' WHERE id = ?")
        .bind(&kept)
        .execute(&app.state.db)
        .await
        .unwrap();
    app.call(
        "POST",
        "/api/worker/v1/heartbeat",
        &token,
        Some(json!({"current_job_id": kept, "gpu": {"utilization": 55.0}})),
    )
    .await;
    recover_jobs(&app.state.db, None, "lease expired")
        .await
        .unwrap();
    assert_eq!(app.job(&alice, &kept).await["status"], "preparing");

    // A cancel request on a job whose worker died resolves to cancelled, not requeued.
    app.call("POST", &format!("/api/v1/jobs/{kept}/cancel"), &alice, None)
        .await;
    app.expire_lease(&kept).await;
    assert_eq!(app.job(&alice, &kept).await["status"], "cancelled");
}

// ---------- node registry & scheduling ----------

#[tokio::test]
async fn capability_based_scheduling() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let alice = app.login("alice").await;
    let other =
        json!([{"engine": "acestep", "key": "acestep-v15-turbo", "name": "ACE-Step 1.5 Turbo"}]);
    let (_, token_a) = app.node(&admin, "render-node-01", mock_models()).await;
    let (id_b, token_b) = app.node(&admin, "render-node-02", other.clone()).await;
    let mock_id = app.model_id(&alice, "mock-v1").await;
    let ace_id = app.model_id(&alice, "acestep-v15-turbo").await;

    // Model registry lists which nodes have which model.
    let models = app.call("GET", "/api/v1/models", &alice, None).await.json();
    let ace = models
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == ace_id.as_str())
        .unwrap();
    assert_eq!(ace["installed_nodes"][0]["node_name"], "render-node-02");
    assert_eq!(ace["installed_nodes"][0]["online"], true);

    // Each job only goes to a node that has its model.
    let ace_job = app.generate(&alice, request_body(&ace_id)).await.json()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let mock_job = app.generate(&alice, request_body(&mock_id)).await.json()["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(app.claim(&token_a).await.json()["id"], mock_job);
    assert_eq!(app.claim(&token_a).await.status, StatusCode::NO_CONTENT);

    // Maintenance and disabled nodes get nothing; the job waits in the queue.
    app.call(
        "PATCH",
        &format!("/api/v1/nodes/{id_b}"),
        &admin,
        Some(json!({"maintenance": true})),
    )
    .await;
    assert_eq!(app.claim(&token_b).await.status, StatusCode::NO_CONTENT);
    app.call(
        "PATCH",
        &format!("/api/v1/nodes/{id_b}"),
        &admin,
        Some(json!({"maintenance": false, "status": "disabled"})),
    )
    .await;
    assert_eq!(app.claim(&token_b).await.code(), "node_disabled");
    assert_eq!(app.job(&alice, &ace_job).await["status"], "queued");
    app.call(
        "PATCH",
        &format!("/api/v1/nodes/{id_b}"),
        &admin,
        Some(json!({"status": "active"})),
    )
    .await;

    // A node that drops a model on re-register stops receiving jobs for it.
    app.register(&token_b, json!([]), false).await;
    assert_eq!(app.claim(&token_b).await.status, StatusCode::NO_CONTENT);
    app.register(&token_b, other, false).await;
    assert_eq!(app.claim(&token_b).await.json()["id"], ace_job);

    // Token rotation locks out the old credential; protocol mismatch is refused.
    let rotated = app
        .call(
            "POST",
            &format!("/api/v1/nodes/{id_b}/rotate-token"),
            &admin,
            None,
        )
        .await
        .json();
    assert_eq!(app.claim(&token_b).await.status, StatusCode::UNAUTHORIZED);
    let new_token = rotated["token"].as_str().unwrap();
    let mismatch = app
        .call(
            "POST",
            "/api/worker/v1/register",
            new_token,
            Some(json!({"protocol_version": 99})),
        )
        .await;
    assert_eq!(mismatch.code(), "protocol_mismatch");
    assert_eq!(
        app.call("POST", "/api/worker/v1/jobs/claim", "", Some(json!({})))
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn concurrent_claims_never_duplicate_a_job() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let alice = app.login("alice").await;
    let mut tokens = Vec::new();
    for i in 0..6 {
        tokens.push(
            app.node(&admin, &format!("render-node-{i:02}"), mock_models())
                .await
                .1,
        );
    }
    let model_id = app.model_id(&alice, "mock-v1").await;
    for _ in 0..3 {
        app.generate(&alice, request_body(&model_id)).await;
    }
    let claims = futures_join(tokens.iter().map(|t| app.claim(t)).collect()).await;
    let mut claimed: Vec<String> = claims
        .iter()
        .filter(|r| r.status == StatusCode::OK)
        .map(|r| r.json()["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(claimed.len(), 3, "three jobs for six nodes");
    claimed.sort();
    claimed.dedup();
    assert_eq!(claimed.len(), 3, "no job handed to two nodes");
}

/// Minimal join-all so the tests need no extra dependency.
async fn futures_join<F: std::future::Future<Output = Reply>>(futures: Vec<F>) -> Vec<Reply> {
    let mut pinned: Vec<_> = futures.into_iter().map(Box::pin).collect();
    let mut out: Vec<Option<Reply>> = pinned.iter().map(|_| None).collect();
    std::future::poll_fn(|cx| {
        let mut pending = false;
        for (i, fut) in pinned.iter_mut().enumerate() {
            if out[i].is_none() {
                match fut.as_mut().poll(cx) {
                    std::task::Poll::Ready(reply) => out[i] = Some(reply),
                    std::task::Poll::Pending => pending = true,
                }
            }
        }
        if pending {
            std::task::Poll::Pending
        } else {
            std::task::Poll::Ready(())
        }
    })
    .await;
    out.into_iter().map(Option::unwrap).collect()
}
