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
        cfg.max_input_audio_mb = 1; // small, so the size limit can be tested
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
        insert_user(&state, "carol", "carol-password".into(), "Carol", "trainer")
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

    /// Create a node that also runs `analyze_track`. Returns (node_id, token).
    async fn analysis_node(&self, admin: &str, name: &str) -> (String, String) {
        let created = self
            .call("POST", "/api/v1/nodes", admin, Some(json!({"name": name})))
            .await;
        let token = created.json()["token"].as_str().unwrap().to_string();
        let body = json!({
            "protocol_version": 1, "worker_version": "test", "fresh_start": true,
            "job_types": ["generate", "analyze_track"],
            "engines": [{"name": "mock", "version": "1", "capabilities": ["generate"]}],
            "models": mock_models(), "loras": [], "gpu": null,
        });
        let reply = self
            .call("POST", "/api/worker/v1/register", &token, Some(body))
            .await;
        assert_eq!(reply.status, StatusCode::OK);
        (reply.json()["node_id"].as_str().unwrap().to_string(), token)
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

    /// Upload an input audio file as a logged-in user.
    async fn upload_input(&self, cookie: &str, file_name: &str, data: &[u8]) -> Reply {
        let boundary = "lushtestboundary";
        let mut body = Vec::new();
        body.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{file_name}\"\r\nContent-Type: application/octet-stream\r\n\r\n").bytes());
        body.extend_from_slice(data);
        body.extend(format!("\r\n--{boundary}--\r\n").bytes());
        let mut request = Request::builder()
            .method("POST")
            .uri("/api/v1/audio-inputs")
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            );
        if !cookie.is_empty() {
            request = request.header(header::COOKIE, cookie);
        }
        self.send(request.body(Body::from(body)).unwrap()).await
    }

    /// Upload one song into a catalog as a logged-in user.
    async fn upload_track(
        &self,
        cookie: &str,
        catalog_id: &str,
        file_name: &str,
        data: &[u8],
    ) -> Reply {
        let boundary = "lushtestboundary";
        let mut body = Vec::new();
        body.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{file_name}\"\r\nContent-Type: application/octet-stream\r\n\r\n").bytes());
        body.extend_from_slice(data);
        body.extend(format!("\r\n--{boundary}--\r\n").bytes());
        let request = Request::builder()
            .method("POST")
            .uri(format!("/api/v1/catalogs/{catalog_id}/tracks"))
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .header(header::COOKIE, cookie);
        self.send(request.body(Body::from(body)).unwrap()).await
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
            Some(json!({"username": "dave", "password": "dave-password", "role": "trainer"})),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let dave_id = created.json()["id"].as_str().unwrap().to_string();
    let dup = app
        .call(
            "POST",
            "/api/v1/users",
            &admin,
            Some(json!({"username": "DAVE", "password": "dave-password", "role": "member"})),
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
    let dave = app.login("dave").await;
    let off = app
        .call(
            "PATCH",
            &format!("/api/v1/users/{dave_id}"),
            &admin,
            Some(json!({"is_active": false})),
        )
        .await;
    assert_eq!(off.json()["is_active"], false);
    assert_eq!(
        app.call("GET", "/api/v1/auth/me", &dave, None).await.status,
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

    // Duration range: seeded genres carry their old default at both ends, a new genre has none
    // until it is set, and the range is validated on write (complete, ordered, 10–300 s).
    assert!(
        genres
            .iter()
            .all(|g| g["duration_min"].is_i64() && g["duration_min"] == g["duration_max"])
    );
    assert!(synth.json()["duration_min"].is_null());
    assert!(synth.json()["duration_max"].is_null());
    let ranged = app
        .call(
            "PATCH",
            &format!("/api/v1/genres/{synth_id}"),
            &admin,
            Some(json!({"duration_min": 90, "duration_max": 300})),
        )
        .await;
    assert_eq!(ranged.status, StatusCode::OK);
    assert_eq!(ranged.json()["duration_min"], 90);
    assert_eq!(ranged.json()["duration_max"], 300);
    for bad in [
        json!({"duration_min": 200, "duration_max": 100}),
        json!({"duration_min": 5}),
        json!({"duration_max": 301}),
        json!({"duration_max": null}),
    ] {
        assert_eq!(
            app.call(
                "PATCH",
                &format!("/api/v1/genres/{synth_id}"),
                &admin,
                Some(bad)
            )
            .await
            .status,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }

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
    // A preset's duration range is optional, but never half filled or reversed.
    assert!(cleared["duration_min"].is_null() && cleared["duration_max"].is_null());
    let ranged_preset = app
        .call(
            "PATCH",
            &format!("/api/v1/presets/{preset_id}"),
            &admin,
            Some(json!({"duration_min": 60, "duration_max": 120})),
        )
        .await;
    assert_eq!(ranged_preset.status, StatusCode::OK);
    assert_eq!(ranged_preset.json()["duration_max"], 120);
    for bad in [
        json!({"duration_min": null}),
        json!({"duration_min": 150}),
        json!({"duration_max": 400}),
    ] {
        assert_eq!(
            app.call(
                "PATCH",
                &format!("/api/v1/presets/{preset_id}"),
                &admin,
                Some(bad)
            )
            .await
            .status,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    let no_range = app
        .call(
            "PATCH",
            &format!("/api/v1/presets/{preset_id}"),
            &admin,
            Some(json!({"duration_min": null, "duration_max": null})),
        )
        .await;
    assert_eq!(no_range.status, StatusCode::OK);
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

    // Archive: every file of the track in one ZIP, byte-identical to the originals.
    let archive_url = format!("/api/v1/tracks/{track_id}/archive");
    let archive = app.call("GET", &archive_url, &alice, None).await;
    assert_eq!(archive.status, StatusCode::OK);
    assert_eq!(archive.headers[header::CONTENT_TYPE], "application/zip");
    let disposition = archive.headers[header::CONTENT_DISPOSITION]
        .to_str()
        .unwrap();
    assert!(disposition.starts_with("attachment") && disposition.ends_with(".zip\""));
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive.bytes.clone())).unwrap();
    assert_eq!(zip.len(), artifacts.len());
    let mut wav = Vec::new();
    std::io::Read::read_to_end(
        &mut zip.by_name(master["filename"].as_str().unwrap()).unwrap(),
        &mut wav,
    )
    .unwrap();
    assert_eq!(wav, WAV);
    assert_eq!(
        app.call("GET", &archive_url, "", None).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.call("GET", "/api/v1/tracks/no-such-track/archive", &alice, None)
            .await
            .status,
        StatusCode::NOT_FOUND
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
    assert_eq!(
        app.call("GET", &archive_url, &alice, None).await.status,
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
        json!({"prompt": "x", "model_id": model_id, "duration": 301}),
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
async fn list_jobs_accepts_comma_separated_types() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let alice = app.login("alice").await;
    app.node(&admin, "render-node-01", mock_models()).await;
    let model_id = app.model_id(&alice, "mock-v1").await;
    app.generate(&alice, request_body(&model_id)).await;
    let count = |path: &'static str| {
        let (app, alice) = (&app, &alice);
        async move {
            app.call("GET", path, alice, None)
                .await
                .json()
                .as_array()
                .unwrap()
                .len()
        }
    };
    assert_eq!(count("/api/v1/jobs?type=generate").await, 1);
    assert_eq!(count("/api/v1/jobs?type=analyze_track,generate").await, 1);
    assert_eq!(
        count("/api/v1/jobs?type=analyze_track,caption_tracks").await,
        0
    );
}

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

// ---------- reference audio & cover ----------

fn audio_models() -> Value {
    json!([
        {"engine": "acestep", "key": "ace-turbo", "name": "ACE Turbo", "capabilities": ["generate", "lora", "reference_audio", "cover"]},
        {"engine": "mock", "key": "mock-v1", "name": "Mock Engine v1", "capabilities": ["generate"]},
    ])
}

#[tokio::test]
async fn audio_input_upload_rules() {
    let app = TestApp::new().await;
    let alice = app.login("alice").await;
    let bob = app.login("bob").await;
    let admin = app.login("admin").await;

    assert_eq!(
        app.upload_input("", "ref.wav", WAV).await.status,
        StatusCode::UNAUTHORIZED
    );
    // The extension and the first bytes must both say WAV / MP3 / FLAC.
    for (name, data) in [
        ("notes.txt", WAV),
        ("ref.wav", b"not really a wav file".as_slice()),
        ("ref.mp3", WAV),
        ("ref.flac", WAV),
        ("ref.wav", b"".as_slice()),
    ] {
        let reply = app.upload_input(&alice, name, data).await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{name}");
    }
    for (name, data) in [
        ("a.mp3", b"ID3\x04\x00 tagged mp3".as_slice()),
        ("b.MP3", b"\xFF\xFB\x90\x00 raw mpeg frame".as_slice()),
        ("c.flac", b"fLaC\x00\x00\x00\x22 flac stream".as_slice()),
    ] {
        let reply = app.upload_input(&alice, name, data).await;
        assert_eq!(reply.status, StatusCode::CREATED, "{name}");
    }
    // Too large: just over the limit (handler) and far over it (body limit).
    let mut big = WAV.to_vec();
    big.resize(1024 * 1024 + 1, 0);
    assert_eq!(
        app.upload_input(&alice, "big.wav", &big).await.status,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    big.resize(3 * 1024 * 1024, 0);
    assert_eq!(
        app.upload_input(&alice, "huge.wav", &big).await.status,
        StatusCode::PAYLOAD_TOO_LARGE
    );

    // A good upload: metadata only, the client file name is kept for display but never used as a path.
    let reply = app
        .upload_input(&alice, "..\\..\\My Reference.WAV", WAV)
        .await;
    assert_eq!(reply.status, StatusCode::CREATED);
    let input = reply.json();
    let id = input["id"].as_str().unwrap();
    assert_eq!(input["filename"], "My Reference.wav");
    assert_eq!(input["content_type"], "audio/wav");
    assert_eq!(input["size_bytes"], WAV.len());
    assert_eq!(input["sha256"], sha256_hex(WAV));
    assert!(input.get("storage_key").is_none() && input.get("owner_id").is_none());
    assert!(
        app.state
            .cfg
            .data_dir
            .join(format!("inputs/{id}.wav"))
            .is_file()
    );
    // Nothing is left behind by the rejected uploads.
    assert_eq!(
        std::fs::read_dir(app.state.cfg.data_dir.join("tmp"))
            .unwrap()
            .count(),
        0
    );

    // Owner and admin can read the metadata and listen; another member cannot use the file.
    let url = format!("/api/v1/audio-inputs/{id}");
    assert_eq!(app.call("GET", &url, &alice, None).await.json()["id"], id);
    assert_eq!(
        app.call("GET", &url, &admin, None).await.status,
        StatusCode::OK
    );
    assert_eq!(
        app.call("GET", &url, &bob, None).await.status,
        StatusCode::NOT_FOUND
    );
    let content = app
        .call(
            "GET",
            &format!("/api/v1/artifacts/{id}/content"),
            &alice,
            None,
        )
        .await;
    assert_eq!(content.bytes, WAV);
}

#[tokio::test]
async fn reference_and_cover_generation() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let alice = app.login("alice").await;
    let bob = app.login("bob").await;
    let (_, token) = app.node(&admin, "render-node-01", audio_models()).await;
    let ace = app.model_id(&alice, "ace-turbo").await;
    let mock = app.model_id(&alice, "mock-v1").await;
    let input = app.upload_input(&alice, "ref.wav", WAV).await.json();
    let input_id = input["id"].as_str().unwrap().to_string();
    let body = |extra: Value| {
        let mut body = json!({"prompt": "Dark ambient drone", "model_id": ace, "duration": 30});
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        body
    };

    // Validation: unknown mode, missing file, file of the wrong mode, someone else's file,
    // a model without the capability, a bad strength.
    for (who, bad) in [
        (&alice, body(json!({"mode": "extend"}))),
        (&alice, body(json!({"mode": "reference"}))),
        (&alice, body(json!({"mode": "cover"}))),
        (
            &alice,
            body(json!({"mode": "cover", "reference_audio_id": input_id})),
        ),
        (
            &alice,
            body(
                json!({"mode": "reference", "reference_audio_id": input_id, "source_audio_id": input_id}),
            ),
        ),
        (&alice, body(json!({"reference_audio_id": input_id}))),
        (
            &alice,
            body(json!({"mode": "reference", "reference_audio_id": "no-such-file"})),
        ),
        (
            &bob,
            body(json!({"mode": "reference", "reference_audio_id": input_id})),
        ),
        (
            &alice,
            body(json!({"mode": "reference", "reference_audio_id": input_id, "model_id": mock})),
        ),
        (
            &alice,
            body(json!({"mode": "cover", "source_audio_id": input_id, "model_id": mock})),
        ),
        (
            &alice,
            body(
                json!({"mode": "cover", "source_audio_id": input_id, "params": {"audio_cover_strength": 1.5}}),
            ),
        ),
    ] {
        let reply = app.generate(who, bad.clone()).await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    }

    // Text mode is unchanged: no inputs, no mode-only parameters.
    let text = app
        .generate(
            &alice,
            body(json!({"params": {"audio_cover_strength": 0.4}})),
        )
        .await;
    assert_eq!(text.status, StatusCode::CREATED);
    let claim = app.claim(&token).await.json();
    assert_eq!(claim["payload"]["mode"], "text");
    assert_eq!(claim["payload"]["inputs"], json!({}));
    assert_eq!(claim["payload"]["duration"], 30);
    assert_eq!(claim["payload"]["params"], json!({}));
    app.call(
        "POST",
        &format!("/api/worker/v1/jobs/{}/fail", claim["id"].as_str().unwrap()),
        &token,
        Some(json!({"lease_token": claim["lease_token"], "error": "skip"})),
    )
    .await;

    // Reference to Music: the payload names the file; only the node holding the job can fetch it.
    let created = app
        .generate(
            &alice,
            body(json!({"mode": "reference", "reference_audio_id": input_id})),
        )
        .await;
    assert_eq!(
        created.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&created.bytes)
    );
    let job_id = created.json()["id"].as_str().unwrap().to_string();
    assert_eq!(created.json()["request"]["mode"], "reference");
    let input_url = |job: &str, artifact: &str, lease: &str| {
        format!("/api/worker/v1/jobs/{job}/inputs/{artifact}?lease_token={lease}")
    };
    assert_eq!(
        app.call("GET", &input_url(&job_id, &input_id, "x"), &token, None)
            .await
            .code(),
        "lease_lost",
        "a queued job has no holder"
    );
    let claim = app.claim(&token).await.json();
    let lease = claim["lease_token"].as_str().unwrap().to_string();
    let payload = &claim["payload"];
    assert_eq!(payload["mode"], "reference");
    assert_eq!(payload["duration"], 30);
    assert_eq!(
        payload["inputs"],
        json!({"reference_audio": {"artifact_id": input_id, "filename": "ref.wav", "size_bytes": WAV.len(), "sha256": sha256_hex(WAV)}})
    );
    let file = app
        .call("GET", &input_url(&job_id, &input_id, &lease), &token, None)
        .await;
    assert_eq!(file.status, StatusCode::OK);
    assert_eq!(file.bytes, WAV);
    assert_eq!(
        app.call("GET", &input_url(&job_id, &input_id, "wrong"), &token, None)
            .await
            .code(),
        "lease_lost"
    );
    // A file that is not an input of this job is not served, even though it exists.
    let other = app.upload_input(&alice, "other.wav", WAV).await.json();
    assert_eq!(
        app.call(
            "GET",
            &input_url(&job_id, other["id"].as_str().unwrap(), &lease),
            &token,
            None
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.call("GET", &input_url(&job_id, &input_id, &lease), &alice, None)
            .await
            .status,
        StatusCode::UNAUTHORIZED,
        "the worker endpoint does not accept a user session"
    );

    // Completion: the library track records the mode and the input it was made from.
    let sha = sha256_hex(WAV);
    assert_eq!(
        app.upload(
            &token,
            &job_id,
            &lease,
            "audio_master",
            "master.wav",
            WAV,
            &sha
        )
        .await
        .status,
        StatusCode::OK
    );
    let done = app
        .call(
            "POST",
            &format!("/api/worker/v1/jobs/{job_id}/complete"),
            &token,
            Some(json!({"lease_token": lease, "result": {"seed": 7, "duration": 30.0, "sample_rate": 48000, "engine": "acestep", "model_key": "ace-turbo", "engine_info": {"task_type": "text2music"}}})),
        )
        .await;
    assert_eq!(done.status, StatusCode::OK);
    let track_id = app.job(&alice, &job_id).await["track_id"]
        .as_str()
        .unwrap()
        .to_string();
    let track = app
        .call("GET", &format!("/api/v1/tracks/{track_id}"), &alice, None)
        .await
        .json();
    assert_eq!(track["params"]["mode"], "reference");
    assert_eq!(
        track["params"]["inputs"]["reference_audio"]["artifact_id"],
        input_id
    );
    // The input is not one of the track's own files (it is not in the download ZIP).
    assert_eq!(track["artifacts"].as_array().unwrap().len(), 1);
    assert_eq!(app.claim(&token).await.status, StatusCode::NO_CONTENT);

    // Cover / Remix: length follows the source, strength is passed through; retry keeps the input.
    let cover = app
        .generate(
            &alice,
            body(
                json!({"mode": "cover", "source_audio_id": input_id, "duration": 0, "params": {"audio_cover_strength": 0.6}}),
            ),
        )
        .await;
    assert_eq!(
        cover.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&cover.bytes)
    );
    let cover_id = cover.json()["id"].as_str().unwrap().to_string();
    let claim = app.claim(&token).await.json();
    let payload = &claim["payload"];
    assert_eq!(payload["mode"], "cover");
    assert!(payload["duration"].is_null());
    assert_eq!(payload["params"]["audio_cover_strength"], 0.6);
    assert_eq!(payload["inputs"]["source_audio"]["artifact_id"], input_id);
    assert!(payload["inputs"].get("reference_audio").is_none());
    app.call(
        "POST",
        &format!("/api/worker/v1/jobs/{cover_id}/fail"),
        &token,
        Some(json!({"lease_token": claim["lease_token"], "error": "engine rejected the task"})),
    )
    .await;
    assert_eq!(app.job(&alice, &cover_id).await["status"], "failed");
    let retried = app
        .call(
            "POST",
            &format!("/api/v1/jobs/{cover_id}/retry"),
            &alice,
            None,
        )
        .await;
    assert_eq!(retried.status, StatusCode::CREATED);
    let claim = app.claim(&token).await.json();
    assert_eq!(claim["id"], retried.json()["id"]);
    assert_eq!(
        claim["payload"]["inputs"]["source_audio"]["artifact_id"],
        input_id
    );
}

// ---------- AI Training (M1) ----------

fn training_body(model_id: &str) -> Value {
    json!({
        "name": "Lo-fi Jazz v1", "dataset_key": "jazz/v1", "base_model_id": model_id,
        "rank": 32, "alpha": 64, "learning_rate": 1e-4, "epochs": 4, "gradient_accumulation": 4,
        "save_every": 2, "warmup_steps": 2, "seed": 42, "max_minutes": 30,
    })
}

#[tokio::test]
async fn training_run_flow() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let alice = app.login("alice").await;
    let created = app
        .call(
            "POST",
            "/api/v1/nodes",
            &admin,
            Some(json!({"name": "train-node"})),
        )
        .await;
    let token = created.json()["token"].as_str().unwrap().to_string();
    let register = |datasets: Value| {
        json!({
            "protocol_version": 1, "worker_version": "test", "fresh_start": true,
            "job_types": ["generate", "train_lora"],
            "engines": [{"name": "acestep", "version": "1", "capabilities": ["generate", "lora"]}],
            "models": [{"engine": "acestep", "key": "acestep-v15-turbo", "capabilities": ["generate", "lora"]}],
            "loras": [], "datasets": datasets, "gpu": null,
        })
    };
    let reply = app
        .call(
            "POST",
            "/api/worker/v1/register",
            &token,
            Some(register(
                json!([{"key": "jazz/v1", "samples": 8, "seconds": 480}]),
            )),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let model_id = app.model_id(&admin, "acestep-v15-turbo").await;

    // Datasets are what the node reports; only admins see them or start a run.
    let datasets = app
        .call("GET", "/api/v1/datasets", &admin, None)
        .await
        .json();
    assert_eq!(datasets[0]["key"], "jazz/v1");
    assert_eq!(datasets[0]["can_train"], true);
    assert_eq!(datasets[0]["info"]["samples"], 8);
    let runs = "/api/v1/training-runs";
    let denied = app
        .call("POST", runs, &alice, Some(training_body(&model_id)))
        .await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);

    let mut bad = training_body(&model_id);
    bad["rank"] = json!(0);
    assert_eq!(
        app.call("POST", runs, &admin, Some(bad)).await.status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let mut bad = training_body(&model_id);
    bad["dataset_key"] = json!("missing/v1");
    assert_eq!(
        app.call("POST", runs, &admin, Some(bad)).await.status,
        StatusCode::UNPROCESSABLE_ENTITY
    );

    let run = app
        .call("POST", runs, &admin, Some(training_body(&model_id)))
        .await;
    assert_eq!(
        run.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&run.bytes)
    );
    let run = run.json();
    let run_id = run["id"].as_str().unwrap().to_string();
    assert_eq!(run["type"], "train_lora");
    assert_eq!(run["max_attempts"], 1);
    // Only one run at a time.
    let second = app
        .call("POST", runs, &admin, Some(training_body(&model_id)))
        .await;
    assert_eq!(second.status, StatusCode::CONFLICT);

    // A waiting generate job goes first even though the training run is older.
    let generation = app.generate(&alice, request_body(&model_id)).await.json();
    let first = app.claim(&token).await.json();
    assert_eq!(first["id"], generation["id"]);
    let failed = app
        .call(
            "POST",
            &format!("/api/worker/v1/jobs/{}/fail", first["id"].as_str().unwrap()),
            &token,
            Some(json!({"lease_token": first["lease_token"], "error": "test"})),
        )
        .await;
    assert_eq!(failed.status, StatusCode::OK);

    let claim = app.claim(&token).await.json();
    assert_eq!(claim["id"], run_id.as_str());
    assert_eq!(claim["type"], "train_lora");
    assert_eq!(claim["payload"]["dataset"]["key"], "jazz/v1");
    assert_eq!(claim["payload"]["config"]["rank"], 32);
    assert_eq!(claim["payload"]["limits"]["max_seconds"], 1800);
    let lora_key = claim["payload"]["lora"]["key"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(lora_key.starts_with("lo-fi-jazz-v1-"), "{lora_key}");
    let lease = claim["lease_token"].clone();

    // Progress carries a phase and structured metrics; a repeated sequence is not stored twice.
    let progress = format!("/api/worker/v1/jobs/{run_id}/progress");
    for _ in 0..2 {
        let reply = app
            .call(
                "POST",
                &progress,
                &token,
                Some(json!({
                    "lease_token": lease, "progress": 0.3, "stage": "running", "phase": "training",
                    "message": "Epoch 1/4",
                    "metrics": [
                        {"sequence": 1, "global_step": 1, "epoch": 1, "train_loss": 0.83, "elapsed_seconds": 3.0},
                        {"sequence": 2, "global_step": 2, "epoch": 1, "train_loss": 0.79, "gpu_memory_mb": 5300},
                    ],
                })),
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK);
    }
    let detail = app
        .call("GET", &format!("{runs}/{run_id}"), &admin, None)
        .await
        .json();
    assert_eq!(detail["status"], "running");
    assert_eq!(detail["stage"], "training");
    assert_eq!(detail["metrics"].as_array().unwrap().len(), 2);
    assert_eq!(detail["metrics"][1]["train_loss"], 0.79);
    assert!(detail["metrics"][0]["learning_rate"].is_null());
    assert!(detail["lora"].is_null());
    let hidden = app
        .call("GET", &format!("{runs}/{run_id}"), &alice, None)
        .await;
    assert_eq!(hidden.status, StatusCode::FORBIDDEN);
    let summary = app
        .call("GET", "/api/v1/queue/summary", &alice, None)
        .await
        .json();
    assert_eq!(summary["training"]["job_id"], run_id.as_str());
    assert_eq!(summary["training"]["node_name"], "train-node");

    // Completing needs the adapter checksum; it puts the LoRA in the registry under evaluation.
    let complete = format!("/api/worker/v1/jobs/{run_id}/complete");
    let missing = app
        .call(
            "POST",
            &complete,
            &token,
            Some(json!({"lease_token": lease, "result": {}})),
        )
        .await;
    assert_eq!(missing.code(), "missing_artifact");
    let done = app
        .call(
            "POST",
            &complete,
            &token,
            Some(json!({"lease_token": lease, "result": {"lora": {"adapter_sha256": "abc123", "bytes": 44}}})),
        )
        .await;
    assert_eq!(
        done.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&done.bytes)
    );
    let summary = app
        .call("GET", "/api/v1/queue/summary", &alice, None)
        .await
        .json();
    assert!(summary["training"].is_null());

    let loras = app.call("GET", "/api/v1/loras", &alice, None).await.json();
    let lora = &loras[0];
    let lora_id = lora["id"].as_str().unwrap().to_string();
    assert_eq!(lora["key"], lora_key.as_str());
    assert_eq!(lora["status"], "evaluation");
    assert_eq!(lora["dataset_key"], "jazz/v1");
    assert_eq!(lora["trainer"], "sidestep");
    assert_eq!(lora["adapter_sha256"], "abc123");
    assert_eq!(lora["config"]["rank"], 32);
    assert_eq!(lora["installed_nodes"][0]["node_name"], "train-node");
    let detail = app
        .call("GET", &format!("{runs}/{run_id}"), &admin, None)
        .await
        .json();
    assert_eq!(detail["lora"]["id"], lora_id.as_str());

    // Under evaluation only an admin may generate with it; after approval everyone can.
    let mut body = request_body(&model_id);
    body["lora_id"] = json!(lora_id);
    assert_eq!(
        app.generate(&alice, body.clone()).await.status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        app.generate(&admin, body.clone()).await.status,
        StatusCode::CREATED
    );
    let patch = format!("/api/v1/loras/{lora_id}");
    let approve = json!({"status": "approved"});
    let forbidden = app
        .call("PATCH", &patch, &alice, Some(approve.clone()))
        .await;
    assert_eq!(forbidden.status, StatusCode::FORBIDDEN);
    let approved = app.call("PATCH", &patch, &admin, Some(approve)).await;
    assert_eq!(approved.json()["status"], "approved");
    assert_eq!(app.generate(&alice, body).await.status, StatusCode::CREATED);
    let invalid = app
        .call("PATCH", &patch, &admin, Some(json!({"status": "training"})))
        .await;
    assert_eq!(invalid.status, StatusCode::UNPROCESSABLE_ENTITY);

    // A training run is never retried; a new run is created instead.
    let retry = app
        .call(
            "POST",
            &format!("/api/v1/jobs/{run_id}/retry"),
            &admin,
            None,
        )
        .await;
    assert_eq!(retry.status, StatusCode::CONFLICT);

    // A worker that does not report datasets leaves the list alone; an empty list clears it.
    let mut old_worker = register(json!([]));
    old_worker.as_object_mut().unwrap().remove("datasets");
    old_worker["fresh_start"] = json!(false);
    app.call("POST", "/api/worker/v1/register", &token, Some(old_worker))
        .await;
    let listed = app
        .call("GET", "/api/v1/datasets", &admin, None)
        .await
        .json();
    assert_eq!(listed.as_array().unwrap().len(), 1);
    let mut cleared = register(json!([]));
    cleared["fresh_start"] = json!(false);
    app.call("POST", "/api/worker/v1/register", &token, Some(cleared))
        .await;
    let listed = app
        .call("GET", "/api/v1/datasets", &admin, None)
        .await
        .json();
    assert!(listed.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn trainer_role_can_use_trainer_routes() {
    let app = TestApp::new().await;
    let carol = app.login("carol").await;
    let alice = app.login("alice").await;
    let admin = app.login("admin").await;
    let reply = app.call("GET", "/api/v1/catalogs", &carol, None).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json(), json!([]));
    let reply = app.call("GET", "/api/v1/catalogs", &alice, None).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.code(), "forbidden");
    let reply = app.call("GET", "/api/v1/catalogs", &admin, None).await;
    assert_eq!(reply.status, StatusCode::OK);
}

#[tokio::test]
async fn captioner_model_reported_with_kind_and_rejected_for_generation() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let alice = app.login("alice").await;
    let models = json!([
        {"engine": "acestep", "key": "acestep-v15-turbo", "capabilities": ["generate"]},
        {"engine": "captioner", "key": "moss-music-8b-instruct", "name": "MOSS-Music 8B", "capabilities": ["caption"]},
    ]);
    app.node(&admin, "kind-node", models).await;
    let list = app.call("GET", "/api/v1/models", &alice, None).await.json();
    let kind_of = |key: &str| {
        list.as_array()
            .unwrap()
            .iter()
            .find(|m| m["key"] == key)
            .unwrap()["kind"]
            .clone()
    };
    assert_eq!(kind_of("moss-music-8b-instruct"), "captioner");
    assert_eq!(kind_of("acestep-v15-turbo"), "generator");
    let captioner_id = app.model_id(&alice, "moss-music-8b-instruct").await;
    let reply = app.generate(&alice, request_body(&captioner_id)).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        reply.json()["error"]["message"],
        "Model này không dùng để tạo nhạc"
    );
}

#[tokio::test]
async fn catalog_crud() {
    let app = TestApp::new().await;
    let carol = app.login("carol").await;
    let alice = app.login("alice").await;
    let genres = app.call("GET", "/api/v1/genres", &carol, None).await.json();
    let lofi_id = genres
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["slug"] == "lo-fi")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let created = app
        .call(
            "POST",
            "/api/v1/catalogs",
            &carol,
            Some(json!({"name": "  Lo-fi chill 2026  ", "genre_id": lofi_id})),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let catalog = created.json();
    assert_eq!(catalog["name"], "Lo-fi chill 2026");
    assert_eq!(catalog["genre_id"], lofi_id.as_str());
    assert_eq!(catalog["genre_name"], "Lo-fi");
    assert_eq!(catalog["track_count"], 0);
    assert_eq!(catalog["total_seconds"].as_f64(), Some(0.0));
    let id = catalog["id"].as_str().unwrap().to_string();

    let dup = app
        .call(
            "POST",
            "/api/v1/catalogs",
            &carol,
            Some(json!({"name": "Lo-fi chill 2026", "genre_id": lofi_id})),
        )
        .await;
    assert_eq!(dup.status, StatusCode::CONFLICT);
    assert_eq!(dup.code(), "conflict");
    let bad_genre = app
        .call(
            "POST",
            "/api/v1/catalogs",
            &carol,
            Some(json!({"name": "Other", "genre_id": "nope"})),
        )
        .await;
    assert_eq!(bad_genre.status, StatusCode::UNPROCESSABLE_ENTITY);
    for name in ["", "   "] {
        let reply = app
            .call(
                "POST",
                "/api/v1/catalogs",
                &carol,
                Some(json!({"name": name, "genre_id": lofi_id})),
            )
            .await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    }
    let long = app
        .call(
            "POST",
            "/api/v1/catalogs",
            &carol,
            Some(json!({"name": "x".repeat(121), "genre_id": lofi_id})),
        )
        .await;
    assert_eq!(long.status, StatusCode::UNPROCESSABLE_ENTITY);

    let forbidden = app
        .call("GET", &format!("/api/v1/catalogs/{id}"), &alice, None)
        .await;
    assert_eq!(forbidden.status, StatusCode::FORBIDDEN);

    let listed = app
        .call("GET", "/api/v1/catalogs", &carol, None)
        .await
        .json();
    assert_eq!(listed.as_array().unwrap().len(), 1);

    let patched = app
        .call(
            "PATCH",
            &format!("/api/v1/catalogs/{id}"),
            &carol,
            Some(json!({"name": "Lo-fi chill 2027"})),
        )
        .await;
    assert_eq!(patched.status, StatusCode::OK);
    assert_eq!(patched.json()["name"], "Lo-fi chill 2027");
    let empty_patch = app
        .call(
            "PATCH",
            &format!("/api/v1/catalogs/{id}"),
            &carol,
            Some(json!({"name": " "})),
        )
        .await;
    assert_eq!(empty_patch.status, StatusCode::UNPROCESSABLE_ENTITY);

    let deleted = app
        .call("DELETE", &format!("/api/v1/catalogs/{id}"), &carol, None)
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    let gone = app
        .call("GET", &format!("/api/v1/catalogs/{id}"), &carol, None)
        .await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
    let again = app
        .call("DELETE", &format!("/api/v1/catalogs/{id}"), &carol, None)
        .await;
    assert_eq!(again.status, StatusCode::NOT_FOUND);
    let listed = app
        .call("GET", "/api/v1/catalogs", &carol, None)
        .await
        .json();
    assert!(listed.as_array().unwrap().is_empty());
    let recreated = app
        .call(
            "POST",
            "/api/v1/catalogs",
            &carol,
            Some(json!({"name": "Lo-fi chill 2027", "genre_id": lofi_id})),
        )
        .await;
    assert_eq!(recreated.status, StatusCode::CREATED);
}

async fn lofi_catalog(app: &TestApp, cookie: &str, name: &str) -> String {
    let genres = app.call("GET", "/api/v1/genres", cookie, None).await.json();
    let genre_id = genres
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["slug"] == "lo-fi")
        .unwrap()["id"]
        .clone();
    let created = app
        .call(
            "POST",
            "/api/v1/catalogs",
            cookie,
            Some(json!({"name": name, "genre_id": genre_id})),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED);
    created.json()["id"].as_str().unwrap().to_string()
}

fn analysis_result(bpm: f64, duration: f64) -> Value {
    json!({
        "duration_s": duration,
        "tempo": {"bpm": bpm, "confidence": 0.82, "alternatives": [bpm * 2.0]},
        "key": {"key": "D minor", "confidence": 0.61},
        "loudness_lufs": -14.2, "clipping": false, "silence_ratio": 0.03,
    })
}

#[tokio::test]
async fn catalog_upload_creates_analyze_job() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    let alice = app.login("alice").await;
    let (_, token) = app.analysis_node(&admin, "analysis-node").await;
    let catalog = lofi_catalog(&app, &carol, "Lo-fi upload").await;
    let tracks_url = format!("/api/v1/catalogs/{catalog}/tracks");

    // Members cannot use the catalog.
    assert_eq!(
        app.upload_track(&alice, &catalog, "a.wav", WAV)
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.upload_track(&carol, "missing", "a.wav", WAV)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.upload_track(&carol, &catalog, "a.txt", WAV)
            .await
            .status,
        StatusCode::UNPROCESSABLE_ENTITY
    );

    let reply = app
        .upload_track(&carol, &catalog, "Rainy Day.WAV", WAV)
        .await;
    assert_eq!(reply.status, StatusCode::CREATED);
    let track = reply.json();
    assert_eq!(track["analysis_status"], "pending");
    assert_eq!(track["filename"], "Rainy Day.wav");
    assert_eq!(track["catalog_id"], catalog.as_str());
    assert!(track["duration_s"].is_null() && track["tempo_bpm"].is_null());
    assert_eq!(track["models_done"], json!([]));
    assert!(track["caption"].is_null() && track["current_model_key"].is_null());
    assert_eq!(track["voice_warning"], false);
    assert!(track["estimates"]["tempo"].is_null() && track["estimates"]["key"].is_null());
    let artifact_id = track["artifact_id"].as_str().unwrap().to_string();

    let job = app.claim(&token).await.json();
    assert_eq!(job["type"], "analyze_track");
    assert_eq!(job["payload"]["catalog_track_id"], track["id"]);
    assert_eq!(
        job["payload"]["inputs"]["track"]["artifact_id"],
        artifact_id.as_str()
    );
    let (job_id, lease) = (
        job["id"].as_str().unwrap().to_string(),
        job["lease_token"].as_str().unwrap().to_string(),
    );
    let input = app
        .call(
            "GET",
            &format!("/api/worker/v1/jobs/{job_id}/inputs/{artifact_id}?lease_token={lease}"),
            &token,
            None,
        )
        .await;
    assert_eq!(input.status, StatusCode::OK);
    assert_eq!(input.bytes, WAV);

    let done = app
        .call(
            "POST",
            &format!("/api/worker/v1/jobs/{job_id}/complete"),
            &token,
            Some(json!({"lease_token": lease, "result": analysis_result(78.0, 123.5)})),
        )
        .await;
    assert_eq!(done.status, StatusCode::OK);

    let list = app.call("GET", &tracks_url, &carol, None).await.json();
    let t = &list[0];
    assert_eq!(t["analysis_status"], "done");
    assert_eq!(t["duration_s"].as_f64(), Some(123.5));
    assert_eq!(t["tempo_bpm"].as_f64(), Some(78.0));
    assert_eq!(t["tempo_source"], "auto");
    assert_eq!(t["key_name"], "D minor");
    assert_eq!(t["key_source"], "auto");
    assert_eq!(t["estimates"]["tempo"]["confidence"].as_f64(), Some(0.82));
    assert_eq!(t["estimates"]["tempo"]["alternatives"], json!([156.0]));
    assert_eq!(t["estimates"]["key"]["key"], "D minor");
    assert_eq!(t["estimates"]["key"]["confidence"].as_f64(), Some(0.61));

    // A second track: the catalog total is the sum of the durations (decodes as f64).
    let other = app
        .upload_track(&carol, &catalog, "b.mp3", b"ID3\x04\x00 another song")
        .await
        .json();
    let job2 = app.claim(&token).await.json();
    app.call(
        "POST",
        &format!(
            "/api/worker/v1/jobs/{}/complete",
            job2["id"].as_str().unwrap()
        ),
        &token,
        Some(json!({"lease_token": job2["lease_token"], "result": analysis_result(90.0, 60.25)})),
    )
    .await;
    assert_eq!(other["analysis_status"], "pending");
    let cat = app
        .call("GET", &format!("/api/v1/catalogs/{catalog}"), &carol, None)
        .await
        .json();
    assert_eq!(cat["track_count"], 2);
    assert_eq!(cat["total_seconds"].as_f64(), Some(183.75));
}

#[tokio::test]
async fn catalog_upload_duplicate_rejected() {
    let app = TestApp::new().await;
    let carol = app.login("carol").await;
    let catalog = lofi_catalog(&app, &carol, "Dup A").await;
    let other_catalog = lofi_catalog(&app, &carol, "Dup B").await;
    let first = app
        .upload_track(&carol, &catalog, "Original.wav", WAV)
        .await;
    assert_eq!(first.status, StatusCode::CREATED);
    let id = first.json()["id"].as_str().unwrap().to_string();

    let again = app.upload_track(&carol, &catalog, "Copy.wav", WAV).await;
    assert_eq!(again.status, StatusCode::CONFLICT);
    assert!(
        again.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Original.wav")
    );
    // The rejected upload leaves no artifact, job or file behind.
    let (artifacts, jobs): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM artifacts WHERE kind = 'audio_input'), (SELECT COUNT(*) FROM jobs)",
    )
    .fetch_one(&app.state.db)
    .await
    .unwrap();
    assert_eq!((artifacts, jobs), (1, 1));
    assert_eq!(
        std::fs::read_dir(app.state.cfg.data_dir.join("inputs"))
            .unwrap()
            .count(),
        1
    );

    // Soft-deleting the track frees the file for a new upload.
    sqlx::query("UPDATE catalog_tracks SET deleted_at = '2026-01-01T00:00:00.000Z' WHERE id = ?")
        .bind(&id)
        .execute(&app.state.db)
        .await
        .unwrap();
    assert_eq!(
        app.upload_track(&carol, &catalog, "Original.wav", WAV)
            .await
            .status,
        StatusCode::CREATED
    );
    // The same bytes in another catalog are fine.
    assert_eq!(
        app.upload_track(&carol, &other_catalog, "Original.wav", WAV)
            .await
            .status,
        StatusCode::CREATED
    );
}

#[tokio::test]
async fn analyze_does_not_overwrite_human_values() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    let (_, token) = app.analysis_node(&admin, "analysis-node").await;
    let catalog = lofi_catalog(&app, &carol, "Human").await;
    let track = app
        .upload_track(&carol, &catalog, "h.wav", WAV)
        .await
        .json();
    sqlx::query("UPDATE catalog_tracks SET tempo_bpm = 70.0, tempo_source = 'human' WHERE id = ?")
        .bind(track["id"].as_str().unwrap())
        .execute(&app.state.db)
        .await
        .unwrap();
    let job = app.claim(&token).await.json();
    app.call(
        "POST",
        &format!(
            "/api/worker/v1/jobs/{}/complete",
            job["id"].as_str().unwrap()
        ),
        &token,
        Some(json!({"lease_token": job["lease_token"], "result": analysis_result(78.0, 100.0)})),
    )
    .await;
    let list = app
        .call(
            "GET",
            &format!("/api/v1/catalogs/{catalog}/tracks"),
            &carol,
            None,
        )
        .await
        .json();
    let t = &list[0];
    assert_eq!(t["tempo_bpm"].as_f64(), Some(70.0));
    assert_eq!(t["tempo_source"], "human");
    // The key had no human value, so the measurement fills it; the estimate is still reported.
    assert_eq!(t["key_name"], "D minor");
    assert_eq!(t["key_source"], "auto");
    assert_eq!(t["estimates"]["tempo"]["bpm"].as_f64(), Some(78.0));
}

#[tokio::test]
async fn analyze_track_claim_order() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    let alice = app.login("alice").await;
    let (_, token) = app.analysis_node(&admin, "analysis-node").await;
    let catalog = lofi_catalog(&app, &carol, "Order").await;
    // The analysis job is queued first, yet the generation job runs first.
    app.upload_track(&carol, &catalog, "o.wav", WAV).await;
    let mock_id = app.model_id(&alice, "mock-v1").await;
    let generate = app.generate(&alice, request_body(&mock_id)).await.json();
    let first = app.claim(&token).await.json();
    assert_eq!(first["id"], generate["id"]);
    app.call(
        "POST",
        &format!(
            "/api/worker/v1/jobs/{}/complete",
            first["id"].as_str().unwrap()
        ),
        &token,
        Some(json!({"lease_token": first["lease_token"], "result": {}})),
    )
    .await;
}

#[tokio::test]
async fn analysis_status_failed_is_derived_from_the_job() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    let (_, token) = app.analysis_node(&admin, "analysis-node").await;
    let catalog = lofi_catalog(&app, &carol, "Failing").await;
    let tracks_url = format!("/api/v1/catalogs/{catalog}/tracks");
    let track = app
        .upload_track(&carol, &catalog, "f.wav", WAV)
        .await
        .json();
    let job_id = {
        let (id,): (String,) =
            sqlx::query_as("SELECT analysis_job_id FROM catalog_tracks WHERE id = ?")
                .bind(track["id"].as_str().unwrap())
                .fetch_one(&app.state.db)
                .await
                .unwrap();
        id
    };
    // Lease lost three times: attempts run out and the job fails.
    for _ in 0..3 {
        assert_eq!(app.claim(&token).await.json()["id"], job_id.as_str());
        let list = app.call("GET", &tracks_url, &carol, None).await.json();
        assert_eq!(list[0]["analysis_status"], "pending");
        app.expire_lease(&job_id).await;
    }
    assert_eq!(app.job(&carol, &job_id).await["status"], "failed");
    let list = app.call("GET", &tracks_url, &carol, None).await.json();
    assert_eq!(list[0]["analysis_status"], "failed");
    // Nothing was written to the row itself.
    let (stored,): (String,) = sqlx::query_as("SELECT analysis_status FROM catalog_tracks")
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    assert_eq!(stored, "pending");
}

// ---------- Catalog track edits and manual captions ----------

/// Register a node that exposes two captioner models (`cap-x`, `cap-y`) and a generator.
async fn captioner_node(app: &TestApp, admin: &str) {
    let models = json!([
        {"engine": "mock", "key": "mock-v1", "name": "Mock Engine v1", "capabilities": ["generate"]},
        {"engine": "captioner", "key": "cap-x", "name": "Captioner X", "capabilities": ["caption"]},
        {"engine": "captioner", "key": "cap-y", "name": "Captioner Y", "capabilities": ["caption"]},
    ]);
    app.node(admin, "cap-node", models).await;
}

fn caption_body(caption: &str) -> Value {
    json!({"mood": "calm", "instruments": "piano", "vocals": "Instrumental", "caption": caption})
}

async fn put_caption(app: &TestApp, cookie: &str, track: &str, model: &str, body: Value) -> Reply {
    app.call(
        "PUT",
        &format!("/api/v1/catalog-tracks/{track}/captions/{model}"),
        cookie,
        Some(body),
    )
    .await
}

async fn first_track(app: &TestApp, cookie: &str, catalog: &str) -> Value {
    app.call(
        "GET",
        &format!("/api/v1/catalogs/{catalog}/tracks"),
        cookie,
        None,
    )
    .await
    .json()[0]
        .clone()
}

#[tokio::test]
async fn caption_revisions_undo_redo() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    captioner_node(&app, &admin).await;
    let catalog = lofi_catalog(&app, &carol, "Undo").await;
    let track = app
        .upload_track(&carol, &catalog, "u.wav", WAV)
        .await
        .json();
    let id = track["id"].as_str().unwrap();
    let url = |op: &str| format!("/api/v1/catalog-tracks/{id}/captions/cap-x/{op}");

    let a = put_caption(&app, &carol, id, "cap-x", caption_body("A")).await;
    assert_eq!(a.status, StatusCode::OK);
    assert_eq!(a.json()["seq"], 1);
    assert_eq!(a.json()["source"], "human");
    assert_eq!(a.json()["can_undo"], false);
    let b = put_caption(&app, &carol, id, "cap-x", caption_body("B"))
        .await
        .json();
    assert_eq!(
        (b["caption"].as_str(), b["seq"].as_i64()),
        (Some("B"), Some(2))
    );
    assert_eq!(b["can_undo"], true);
    assert_eq!(b["can_redo"], false);

    let undone = app.call("POST", &url("undo"), &carol, None).await;
    assert_eq!(undone.status, StatusCode::OK);
    let undone = undone.json();
    assert_eq!(undone["caption"], "A");
    assert_eq!(undone["can_redo"], true);
    assert_eq!(undone["can_undo"], false);
    let redone = app.call("POST", &url("redo"), &carol, None).await.json();
    assert_eq!(redone["caption"], "B");
    let none_after = app.call("POST", &url("redo"), &carol, None).await;
    assert_eq!(none_after.status, StatusCode::CONFLICT);
    assert_eq!(none_after.json()["error"]["message"], "Không còn bản sau");
    app.call("POST", &url("undo"), &carol, None).await;
    let last = app.call("POST", &url("undo"), &carol, None).await;
    assert_eq!(last.status, StatusCode::CONFLICT);
    assert_eq!(last.json()["error"]["message"], "Không còn bản trước");

    // A new save while the head is behind keeps the later revision.
    let c = put_caption(&app, &carol, id, "cap-x", caption_body("C"))
        .await
        .json();
    assert_eq!(c["seq"], 3);
    assert_eq!(c["can_redo"], false);
    let back = app.call("POST", &url("undo"), &carol, None).await.json();
    assert_eq!(
        (back["caption"].as_str(), back["seq"].as_i64()),
        (Some("B"), Some(2))
    );
    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM caption_revisions")
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    assert_eq!(count, 3);
}

#[tokio::test]
async fn switching_model_keeps_results() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    captioner_node(&app, &admin).await;
    let catalog = lofi_catalog(&app, &carol, "Switch").await;
    let track = app
        .upload_track(&carol, &catalog, "s.wav", WAV)
        .await
        .json();
    let id = track["id"].as_str().unwrap();
    put_caption(&app, &carol, id, "cap-x", caption_body("from x")).await;
    put_caption(&app, &carol, id, "cap-y", caption_body("from y")).await;
    let path = format!("/api/v1/catalog-tracks/{id}");
    let patch = |body: Value| app.call("PATCH", &path, &carol, Some(body));

    let t = patch(json!({"current_model_key": "cap-y"})).await;
    assert_eq!(t.status, StatusCode::OK);
    let t = t.json();
    assert_eq!(t["caption"]["model_key"], "cap-y");
    assert_eq!(t["caption"]["caption"], "from y");
    let t = patch(json!({"current_model_key": "cap-x"})).await.json();
    assert_eq!(t["caption"]["caption"], "from x");
    let mut done: Vec<String> = t["models_done"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    done.sort();
    assert_eq!(done, ["cap-x", "cap-y"]);
    let listed = first_track(&app, &carol, &catalog).await;
    assert_eq!(listed["current_model_key"], "cap-x");
    assert_eq!(listed["caption"]["caption"], "from x");

    for bad in ["mock-v1", "nope"] {
        let r = patch(json!({"current_model_key": bad})).await;
        assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    }
    let r = put_caption(&app, &carol, id, "mock-v1", caption_body("x")).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn catalog_track_tempo_key_edit_delete_and_reanalyze() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    let (_, token) = app.analysis_node(&admin, "analysis-node").await;
    let catalog = lofi_catalog(&app, &carol, "Edit").await;
    let track = app
        .upload_track(&carol, &catalog, "e.wav", WAV)
        .await
        .json();
    let id = track["id"].as_str().unwrap();
    let url = format!("/api/v1/catalog-tracks/{id}");

    let t = app
        .call(
            "PATCH",
            &url,
            &carol,
            Some(json!({"tempo_bpm": 72.5, "key_name": " A minor "})),
        )
        .await
        .json();
    assert_eq!(t["tempo_bpm"].as_f64(), Some(72.5));
    assert_eq!(t["tempo_source"], "human");
    assert_eq!(t["key_name"], "A minor");
    assert_eq!(t["key_source"], "human");
    let t = app
        .call("PATCH", &url, &carol, Some(json!({"tempo_bpm": null})))
        .await
        .json();
    assert_eq!(t["tempo_bpm"], Value::Null);
    assert_eq!(t["tempo_source"], "human");
    assert_eq!(t["key_name"], "A minor");
    for bad in [
        json!({"tempo_bpm": 19}),
        json!({"tempo_bpm": 301}),
        json!({"key_name": "  "}),
        json!({"key_name": "123456789012345678901"}),
    ] {
        let r = app.call("PATCH", &url, &carol, Some(bad.clone())).await;
        assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    }

    // Reanalyze: refused while the first analysis job is still queued, allowed after it failed.
    let reply = app
        .call("POST", &format!("{url}/reanalyze"), &carol, None)
        .await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    let (old_job,): (String,) =
        sqlx::query_as("SELECT analysis_job_id FROM catalog_tracks WHERE id = ?")
            .bind(id)
            .fetch_one(&app.state.db)
            .await
            .unwrap();
    sqlx::query("UPDATE jobs SET status = 'failed' WHERE id = ?")
        .bind(&old_job)
        .execute(&app.state.db)
        .await
        .unwrap();
    assert_eq!(
        first_track(&app, &carol, &catalog).await["analysis_status"],
        "failed"
    );
    let reply = app
        .call("POST", &format!("{url}/reanalyze"), &carol, None)
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json()["analysis_status"], "pending");
    let (new_job,): (String,) =
        sqlx::query_as("SELECT analysis_job_id FROM catalog_tracks WHERE id = ?")
            .bind(id)
            .fetch_one(&app.state.db)
            .await
            .unwrap();
    assert_ne!(new_job, old_job);
    let claimed = app.claim(&token).await.json();
    assert_eq!(claimed["id"], new_job.as_str());
    assert_eq!(claimed["type"], "analyze_track");
    assert_eq!(claimed["payload"]["catalog_track_id"], id);
    assert_eq!(
        claimed["payload"]["inputs"]["track"]["artifact_id"],
        track["artifact_id"]
    );

    // Delete is soft and hides the track everywhere.
    let del = app.call("DELETE", &url, &carol, None).await;
    assert_eq!(del.status, StatusCode::NO_CONTENT);
    let listed = app
        .call(
            "GET",
            &format!("/api/v1/catalogs/{catalog}/tracks"),
            &carol,
            None,
        )
        .await
        .json();
    assert_eq!(listed.as_array().unwrap().len(), 0);
    for (method, path) in [
        ("PATCH", url.clone()),
        ("DELETE", url.clone()),
        ("POST", format!("{url}/reanalyze")),
        ("POST", format!("{url}/captions/cap-x/undo")),
    ] {
        let r = app
            .call(method, &path, &carol, Some(json!({"tempo_bpm": 80})))
            .await;
        assert_eq!(r.status, StatusCode::NOT_FOUND, "{method} {path}");
    }
    let r = put_caption(&app, &carol, id, "cap-x", caption_body("x")).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    let (rows,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM catalog_tracks WHERE id = ?")
        .bind(id)
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    assert_eq!(rows, 1);
}

#[tokio::test]
async fn voice_warning_flags() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    captioner_node(&app, &admin).await;
    let catalog = lofi_catalog(&app, &carol, "Voice").await;
    let track = app
        .upload_track(&carol, &catalog, "v.wav", WAV)
        .await
        .json();
    let id = track["id"].as_str().unwrap();
    assert_eq!(track["voice_warning"], false);
    app.call(
        "PATCH",
        &format!("/api/v1/catalog-tracks/{id}"),
        &carol,
        Some(json!({"current_model_key": "cap-x"})),
    )
    .await;
    let cases = [
        ("Instrumental", "warm pads and brushed drums", false),
        ("Instrumental", "soft hummed vocal line", true),
        ("Instrumental", "humanized drums", false),
        ("", "just piano", false),
        ("female singer", "just piano", true),
    ];
    for (vocals, caption, expected) in cases {
        let body = json!({"mood": "", "instruments": "", "vocals": vocals, "caption": caption});
        put_caption(&app, &carol, id, "cap-x", body).await;
        let t = first_track(&app, &carol, &catalog).await;
        assert_eq!(t["voice_warning"], expected, "{vocals} / {caption}");
    }
}

#[tokio::test]
async fn member_cannot_edit_catalog() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    let alice = app.login("alice").await;
    captioner_node(&app, &admin).await;
    let catalog = lofi_catalog(&app, &carol, "Perm").await;
    let track = app
        .upload_track(&carol, &catalog, "p.wav", WAV)
        .await
        .json();
    let id = track["id"].as_str().unwrap();
    let url = format!("/api/v1/catalog-tracks/{id}");
    let attempts = [
        ("PATCH", url.clone(), Some(json!({"tempo_bpm": 80}))),
        ("DELETE", url.clone(), None),
        ("POST", format!("{url}/reanalyze"), None),
        (
            "PUT",
            format!("{url}/captions/cap-x"),
            Some(caption_body("x")),
        ),
        ("POST", format!("{url}/captions/cap-x/undo"), None),
        ("POST", format!("{url}/captions/cap-x/redo"), None),
    ];
    for (method, path, body) in attempts {
        let r = app.call(method, &path, &alice, body).await;
        assert_eq!(r.status, StatusCode::FORBIDDEN, "{method} {path}");
    }
}

// ---------- caption_tracks jobs ----------

const MOSS: &str = "moss-music-8b-instruct";

/// A node that runs `caption_tracks` and reports the MOSS captioner (when `with_model`).
async fn caption_worker(app: &TestApp, admin: &str, name: &str, with_model: bool) -> String {
    let created = app
        .call("POST", "/api/v1/nodes", admin, Some(json!({"name": name})))
        .await;
    let token = created.json()["token"].as_str().unwrap().to_string();
    register_caption_worker(app, &token, with_model).await;
    token
}

async fn register_caption_worker(app: &TestApp, token: &str, with_model: bool) {
    let models = if with_model {
        json!([{"engine": "captioner", "key": MOSS, "name": "MOSS-Music 8B", "capabilities": ["caption"]}])
    } else {
        json!([])
    };
    let body = json!({
        "protocol_version": 1, "worker_version": "test", "fresh_start": true,
        "job_types": ["caption_tracks"], "engines": [], "models": models, "loras": [], "gpu": null,
    });
    let reply = app
        .call("POST", "/api/worker/v1/register", token, Some(body))
        .await;
    assert_eq!(reply.status, StatusCode::OK);
}

/// Upload three distinct songs; returns their ids in upload order.
async fn three_tracks(app: &TestApp, cookie: &str, catalog: &str) -> Vec<String> {
    let mut ids = Vec::new();
    for i in 0..3 {
        let mut data = WAV.to_vec();
        data.push(b'0' + i);
        let reply = app
            .upload_track(cookie, catalog, &format!("song{i}.wav"), &data)
            .await;
        assert_eq!(reply.status, StatusCode::CREATED);
        ids.push(reply.json()["id"].as_str().unwrap().to_string());
    }
    ids
}

async fn caption_jobs(
    app: &TestApp,
    cookie: &str,
    catalog: &str,
    tracks: &[&String],
    force: bool,
) -> Reply {
    app.call(
        "POST",
        &format!("/api/v1/catalogs/{catalog}/caption-jobs"),
        cookie,
        Some(json!({"model_key": MOSS, "track_ids": tracks, "force": force})),
    )
    .await
}

async fn track_view(app: &TestApp, cookie: &str, catalog: &str, id: &str) -> Value {
    let list = app
        .call(
            "GET",
            &format!("/api/v1/catalogs/{catalog}/tracks"),
            cookie,
            None,
        )
        .await
        .json();
    list.as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == id)
        .unwrap()
        .clone()
}

fn caption_result(lease: &str, track: &str, caption: &str) -> Value {
    json!({
        "lease_token": lease, "catalog_track_id": track, "mood": "calm",
        "instruments": "piano", "vocals": "Instrumental", "caption": caption, "source_ref": "run-1",
    })
}

fn ids(value: &Value) -> Vec<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn caption_job_skips_done_tracks() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    caption_worker(&app, &admin, "cap-worker", true).await;
    let catalog = lofi_catalog(&app, &carol, "Caption jobs").await;
    let t = three_tracks(&app, &carol, &catalog).await;
    assert_eq!(
        put_caption(&app, &carol, &t[0], MOSS, caption_body("done already"))
            .await
            .status,
        StatusCode::OK
    );

    let reply = caption_jobs(&app, &carol, &catalog, &[&t[0], &t[1], &t[2]], false).await;
    assert_eq!(reply.status, StatusCode::OK);
    let body = reply.json();
    assert_eq!(ids(&body["queued"]), vec![t[1].clone(), t[2].clone()]);
    assert_eq!(ids(&body["skipped"]), vec![t[0].clone()]);
    let job = app.job(&carol, body["job_id"].as_str().unwrap()).await;
    assert_eq!(job["type"], "caption_tracks");
    assert_eq!(job["priority"], -20);
    assert_eq!(job["title"], "Mô tả 2 bài · MOSS-Music 8B");
    let (payload, max_attempts, model_id): (String, i64, Option<String>) =
        sqlx::query_as("SELECT payload, max_attempts, model_id FROM jobs WHERE id = ?")
            .bind(body["job_id"].as_str().unwrap())
            .fetch_one(&app.state.db)
            .await
            .unwrap();
    let payload: Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(max_attempts, 2);
    assert_eq!(model_id.as_deref(), payload["model"]["id"].as_str());
    assert_eq!(payload["model"]["key"], MOSS);
    assert_eq!(payload["catalog_id"], catalog.as_str());
    assert_eq!(payload["tracks"].as_array().unwrap().len(), 2);
    assert_eq!(payload["inputs"].as_object().unwrap().len(), 2);
    let first = &payload["tracks"][0];
    assert_eq!(first["catalog_track_id"], t[1].as_str());
    assert!(payload["inputs"][first["input"].as_str().unwrap()]["sha256"].is_string());

    // The model chosen for the run is the one the Tech stack column shows.
    let view = track_view(&app, &carol, &catalog, &t[1]).await;
    assert_eq!(view["current_model_key"], MOSS);
    assert_eq!(view["captioning_model_key"], MOSS);

    // The same tracks again: they wait for the live job, so nothing is queued.
    let again = caption_jobs(&app, &carol, &catalog, &[&t[1], &t[2]], false)
        .await
        .json();
    assert!(again["job_id"].is_null());
    assert_eq!(again["queued"], json!([]));
    assert_eq!(ids(&again["skipped"]), vec![t[1].clone(), t[2].clone()]);

    // Force bypasses the skip.
    let forced = caption_jobs(&app, &carol, &catalog, &[&t[0]], true)
        .await
        .json();
    assert!(forced["job_id"].is_string());
    assert_eq!(ids(&forced["queued"]), vec![t[0].clone()]);

    // Tracks that are not in this catalog are refused.
    let other = lofi_catalog(&app, &carol, "Other catalog").await;
    let bad = caption_jobs(&app, &carol, &other, &[&t[0]], false).await;
    assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn caption_results_per_track_idempotent() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    let token = caption_worker(&app, &admin, "cap-worker", true).await;
    let catalog = lofi_catalog(&app, &carol, "Results").await;
    let t = three_tracks(&app, &carol, &catalog).await;
    let queued = caption_jobs(&app, &carol, &catalog, &[&t[1], &t[2]], false)
        .await
        .json();
    let job = app.claim(&token).await.json();
    assert_eq!(job["id"], queued["job_id"]);
    let (job_id, lease) = (
        job["id"].as_str().unwrap().to_string(),
        job["lease_token"].as_str().unwrap().to_string(),
    );
    let url = format!("/api/worker/v1/jobs/{job_id}/captions");

    let first = app
        .call(
            "POST",
            &url,
            &token,
            Some(caption_result(&lease, &t[1], "soft piano")),
        )
        .await;
    assert_eq!(first.status, StatusCode::OK);
    assert_eq!(first.json()["revision_seq"], 1);
    // A network retry (even with other text) adds nothing and keeps the first revision.
    let repeat = app
        .call(
            "POST",
            &url,
            &token,
            Some(caption_result(&lease, &t[1], "different text")),
        )
        .await;
    assert_eq!(repeat.json()["revision_seq"], 1);
    let (revisions,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM caption_revisions WHERE catalog_track_id = ?")
            .bind(&t[1])
            .fetch_one(&app.state.db)
            .await
            .unwrap();
    assert_eq!(revisions, 1);

    let done = track_view(&app, &carol, &catalog, &t[1]).await;
    assert_eq!(done["caption"]["caption"], "soft piano");
    assert_eq!(done["caption"]["source"], "model");
    assert_eq!(done["caption"]["source_ref"], "run-1");
    assert_eq!(done["models_done"], json!([MOSS]));
    assert!(done["captioning_model_key"].is_null());
    let waiting = track_view(&app, &carol, &catalog, &t[2]).await;
    assert_eq!(waiting["captioning_model_key"], MOSS);
    assert!(waiting["caption"].is_null());

    // A retry must not move a head the user has since changed.
    assert_eq!(
        put_caption(&app, &carol, &t[1], MOSS, caption_body("edited"))
            .await
            .status,
        StatusCode::OK
    );
    let late = app
        .call(
            "POST",
            &url,
            &token,
            Some(caption_result(&lease, &t[1], "soft piano")),
        )
        .await;
    assert_eq!(late.json()["revision_seq"], 1);
    let view = track_view(&app, &carol, &catalog, &t[1]).await;
    assert_eq!(view["caption"]["caption"], "edited");

    // Not part of the job -> 404; wrong lease -> 409.
    let outsider = app
        .call(
            "POST",
            &url,
            &token,
            Some(caption_result(&lease, &t[0], "x")),
        )
        .await;
    assert_eq!(outsider.status, StatusCode::NOT_FOUND);
    let wrong = app
        .call(
            "POST",
            &url,
            &token,
            Some(caption_result("wrong", &t[2], "x")),
        )
        .await;
    assert_eq!(wrong.status, StatusCode::CONFLICT);

    // A per-track error only goes to the log.
    let failed = app
        .call(
            "POST",
            &url,
            &token,
            Some(json!({"lease_token": lease, "catalog_track_id": t[2], "error": "empty caption"})),
        )
        .await;
    assert_eq!(failed.status, StatusCode::OK);
    assert!(failed.json()["revision_seq"].is_null());
    let log = app.job(&carol, &job_id).await["log"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(log.contains("song2.wav: empty caption"), "{log}");
    assert!(
        track_view(&app, &carol, &catalog, &t[2]).await["caption"].is_null(),
        "no revision for a failed track"
    );

    // complete keeps the summary.
    let complete = app
        .call(
            "POST",
            &format!("/api/worker/v1/jobs/{job_id}/complete"),
            &token,
            Some(json!({"lease_token": lease, "result": {"done": 1, "failed": [t[2]]}})),
        )
        .await;
    assert_eq!(complete.status, StatusCode::OK);
    assert_eq!(app.job(&carol, &job_id).await["result"]["done"], 1);
    // The job is over, so the unanswered track no longer shows "being captioned".
    assert!(track_view(&app, &carol, &catalog, &t[2]).await["captioning_model_key"].is_null());
}

#[tokio::test]
async fn caption_job_cancel_keeps_finished_tracks() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    let token = caption_worker(&app, &admin, "cap-worker", true).await;
    let catalog = lofi_catalog(&app, &carol, "Cancel").await;
    let t = three_tracks(&app, &carol, &catalog).await;
    caption_jobs(&app, &carol, &catalog, &[&t[1], &t[2]], false).await;
    let job = app.claim(&token).await.json();
    let (job_id, lease) = (
        job["id"].as_str().unwrap().to_string(),
        job["lease_token"].as_str().unwrap().to_string(),
    );
    let reply = app
        .call(
            "POST",
            &format!("/api/worker/v1/jobs/{job_id}/captions"),
            &token,
            Some(caption_result(&lease, &t[1], "kept")),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);

    let cancel = app
        .call(
            "POST",
            &format!("/api/v1/jobs/{job_id}/cancel"),
            &carol,
            None,
        )
        .await;
    assert_eq!(cancel.status, StatusCode::OK);
    let confirm = app
        .call(
            "POST",
            &format!("/api/worker/v1/jobs/{job_id}/fail"),
            &token,
            Some(json!({"lease_token": lease, "error": "cancelled", "cancelled": true})),
        )
        .await;
    assert_eq!(confirm.json()["status"], "cancelled");

    let kept = track_view(&app, &carol, &catalog, &t[1]).await;
    assert_eq!(kept["caption"]["caption"], "kept");
    assert!(kept["captioning_model_key"].is_null());
    let unfinished = track_view(&app, &carol, &catalog, &t[2]).await;
    assert!(unfinished["captioning_model_key"].is_null());
    assert!(unfinished["caption"].is_null());
}

#[tokio::test]
async fn caption_job_requires_installed_model() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    let token = caption_worker(&app, &admin, "cap-worker", true).await;
    let catalog = lofi_catalog(&app, &carol, "Installed").await;
    let t = three_tracks(&app, &carol, &catalog).await;
    // The node re-registers without the model: it stays in the registry but is installed nowhere.
    register_caption_worker(&app, &token, false).await;
    let reply = caption_jobs(&app, &carol, &catalog, &[&t[0]], false).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        reply.json()["error"]["message"],
        "Model chưa cài trên máy render"
    );
    // Members still cannot use the catalog at all.
    let alice = app.login("alice").await;
    assert_eq!(
        caption_jobs(&app, &alice, &catalog, &[&t[0]], false)
            .await
            .status,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn analyze_complete_ignores_stale_job_and_validates_duration() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    let (_, token) = app.analysis_node(&admin, "analysis-node").await;
    let catalog = lofi_catalog(&app, &carol, "Stale").await;
    let track = app
        .upload_track(&carol, &catalog, "s.wav", WAV)
        .await
        .json();
    let id = track["id"].as_str().unwrap();

    let job = app.claim(&token).await.json();
    let (job_id, lease) = (
        job["id"].as_str().unwrap().to_string(),
        job["lease_token"].as_str().unwrap().to_string(),
    );
    let complete = format!("/api/worker/v1/jobs/{job_id}/complete");

    // A malformed duration is rejected and the job stays open.
    for bad in [json!(-1.0), json!("12"), Value::Null] {
        let mut result = analysis_result(70.0, 10.0);
        result["duration_s"] = bad.clone();
        let r = app
            .call(
                "POST",
                &complete,
                &token,
                Some(json!({"lease_token": lease, "result": result})),
            )
            .await;
        assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    }

    // The track now belongs to a newer analyze job (as after a reanalyze): the old job must not overwrite it.
    sqlx::query("UPDATE catalog_tracks SET analysis_job_id = NULL WHERE id = ?")
        .bind(id)
        .execute(&app.state.db)
        .await
        .unwrap();
    let done = app
        .call(
            "POST",
            &complete,
            &token,
            Some(json!({"lease_token": lease, "result": analysis_result(70.0, 10.0)})),
        )
        .await;
    assert_eq!(done.status, StatusCode::OK);
    let t = first_track(&app, &carol, &catalog).await;
    assert!(t["duration_s"].is_null() && t["tempo_bpm"].is_null());
    assert_eq!(t["analysis_status"], "pending");
}

#[tokio::test]
async fn caption_fields_have_length_caps() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    captioner_node(&app, &admin).await;
    let catalog = lofi_catalog(&app, &carol, "Caps").await;
    let track = app
        .upload_track(&carol, &catalog, "c.wav", WAV)
        .await
        .json();
    let id = track["id"].as_str().unwrap();

    for (field, len) in [
        ("mood", 501),
        ("instruments", 501),
        ("vocals", 501),
        ("caption", 4001),
    ] {
        let mut body = caption_body("ok");
        body[field] = json!("x".repeat(len));
        let r = put_caption(&app, &carol, id, "cap-x", body).await;
        assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{field}");
    }
    let mut body = caption_body(&"c".repeat(4000));
    body["mood"] = json!("m".repeat(500));
    let r = put_caption(&app, &carol, id, "cap-x", body).await;
    assert_eq!(r.status, StatusCode::OK);
}

// ---------- LoRA build preview ----------

fn lora_build_body(model_id: &str, track_ids: &[&String]) -> Value {
    json!({
        "name": "Lo-fi Studio v1", "keyword": "", "test_prompt": "warm lo-fi beat",
        "track_ids": track_ids, "holdout_count": 1, "holdout_seed": 7, "voice_confirmed": [],
        "base_model_id": model_id,
        "rank": 32, "alpha": 64, "learning_rate": 1e-4, "epochs": 4, "gradient_accumulation": 4,
        "save_every": 1, "warmup_steps": 2, "seed": 42, "max_minutes": 30,
    })
}

#[tokio::test]
async fn lora_preview_reports_checks() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    captioner_node(&app, &admin).await;
    let model_id = app.model_id(&admin, "mock-v1").await;
    let cat = lofi_catalog(&app, &carol, "Preview").await;
    let mut ids = three_tracks(&app, &carol, &cat).await;
    let mut data = WAV.to_vec();
    data.push(b'9');
    let fourth = app.upload_track(&carol, &cat, "song9.wav", &data).await;
    assert_eq!(fourth.status, StatusCode::CREATED);
    ids.push(fourth.json()["id"].as_str().unwrap().to_string());
    let (t1, t2, t3, t4) = (&ids[0], &ids[1], &ids[2], &ids[3]);

    for (track, model, text) in [
        (t1, "cap-x", "warm rhodes"),
        (t2, "cap-x", "warm rhodes"),
        (t3, "cap-y", "soft humming over piano"),
    ] {
        let r = put_caption(&app, &carol, track, model, caption_body(text)).await;
        assert_eq!(r.status, StatusCode::OK);
        let r = app
            .call(
                "PATCH",
                &format!("/api/v1/catalog-tracks/{track}"),
                &carol,
                Some(json!({"current_model_key": model})),
            )
            .await;
        assert_eq!(r.status, StatusCode::OK);
    }

    let url = format!("/api/v1/catalogs/{cat}/lora-preview");
    let mut body = lora_build_body(&model_id, &[t1, t2, t3, t4]);
    let p = app.call("POST", &url, &carol, Some(body.clone())).await;
    assert_eq!(p.status, StatusCode::OK);
    let p = p.json();
    assert_eq!(p["ready"], false);
    assert_eq!(p["missing_caption"].as_array().unwrap().len(), 1);
    assert_eq!(p["missing_caption"][0]["id"], t4.as_str());
    assert_eq!(p["voice_unconfirmed"].as_array().unwrap().len(), 1);
    assert_eq!(p["checkpoint_epochs"], json!([1, 2, 3, 4]));

    // Confirm voice for track 3 and drop track 4: 3 tracks, 1 held out, 2 train.
    body["voice_confirmed"] = json!([t3]);
    body["track_ids"] = json!([t1, t2, t3]);
    let p = app
        .call("POST", &url, &carol, Some(body.clone()))
        .await
        .json();
    assert_eq!(p["ready"], true, "{p}");
    assert_eq!(p["errors"], json!([]));
    assert_eq!(p["held_out"].as_array().unwrap().len(), 1);
    assert_eq!(p["train"].as_array().unwrap().len(), 2);
    assert_eq!(p["steps_per_epoch"], 1);
    assert_eq!(p["total_steps"], 4);
    assert_eq!(p["eval_renders"], 4 * (1 + 1) + 1);
    assert_eq!(p["unknown_duration"], 2);
    assert_eq!(p["name_taken"], false);
    assert_eq!(p["no_tempo_key"].as_array().unwrap().len(), 3);
    let models = p["models"].as_array().unwrap();
    assert_eq!(
        models
            .iter()
            .map(|m| m["count"].as_i64().unwrap())
            .sum::<i64>(),
        2
    );
    assert!(
        models
            .windows(2)
            .all(|w| w[0]["count"].as_i64() >= w[1]["count"].as_i64())
    );
    assert!(
        p["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w == "Trong lúc train, máy render tạm không tạo nhạc được")
    );

    // Same seed, same held-out track.
    let again = app
        .call("POST", &url, &carol, Some(body.clone()))
        .await
        .json();
    assert_eq!(again["held_out"], p["held_out"]);

    // Holding out everything leaves nothing to train.
    body["holdout_count"] = json!(3);
    let p = app
        .call("POST", &url, &carol, Some(body.clone()))
        .await
        .json();
    assert_eq!(p["ready"], false);
    assert!(
        p["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e == "Phải còn ít nhất 1 bài để train")
    );
}

#[tokio::test]
async fn lora_preview_checkpoints_and_permissions() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    let alice = app.login("alice").await;
    captioner_node(&app, &admin).await;
    let model_id = app.model_id(&admin, "mock-v1").await;
    let cat = lofi_catalog(&app, &carol, "Preview 2").await;
    let ids = three_tracks(&app, &carol, &cat).await;
    let url = format!("/api/v1/catalogs/{cat}/lora-preview");
    let mut body = lora_build_body(&model_id, &[&ids[0], &ids[1], &ids[2]]);

    assert_eq!(
        app.call("POST", &url, &alice, Some(body.clone()))
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    let missing = app
        .call(
            "POST",
            "/api/v1/catalogs/nope/lora-preview",
            &carol,
            Some(body.clone()),
        )
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);

    let errors = |p: &Value| -> Vec<String> {
        p["errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e.as_str().unwrap().to_string())
            .collect()
    };

    body["epochs"] = json!(5);
    body["save_every"] = json!(2);
    let p = app
        .call("POST", &url, &carol, Some(body.clone()))
        .await
        .json();
    assert_eq!(p["checkpoint_epochs"], json!([2, 4, 5]));

    body["holdout_count"] = json!(6);
    let p = app
        .call("POST", &url, &carol, Some(body.clone()))
        .await
        .json();
    assert!(errors(&p).contains(&"Số bài giữ lại phải từ 0 đến 5".to_string()));
    body["holdout_count"] = json!(0);

    body["keyword"] = json!("lush lofi");
    let p = app
        .call("POST", &url, &carol, Some(body.clone()))
        .await
        .json();
    assert!(errors(&p).contains(&"Từ khóa chỉ gồm chữ, số, _ hoặc -, tối đa 40 ký tự".to_string()));
    body["keyword"] = json!("");

    // Bad config and unknown tracks are hard 422s.
    let mut bad = body.clone();
    bad["rank"] = json!(0);
    assert_eq!(
        app.call("POST", &url, &carol, Some(bad)).await.status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let mut bad = body.clone();
    bad["track_ids"] = json!([ids[0], "ghost"]);
    assert_eq!(
        app.call("POST", &url, &carol, Some(bad)).await.status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let mut bad = body.clone();
    bad["track_ids"] = json!([ids[0], ids[0]]);
    assert_eq!(
        app.call("POST", &url, &carol, Some(bad)).await.status,
        StatusCode::UNPROCESSABLE_ENTITY
    );

    // A name already used by a live LoRA is taken, case-insensitively.
    sqlx::query(
        "INSERT INTO loras (id, key, name, base_model_id, status, created_at) \
         VALUES ('l1', 'lo-fi-studio-v1-aaaa', 'Lo-Fi Studio V1', ?, 'evaluation', '2026-01-01T00:00:00Z')",
    )
    .bind(&model_id)
    .execute(&app.state.db)
    .await
    .unwrap();
    let p = app
        .call("POST", &url, &carol, Some(body.clone()))
        .await
        .json();
    assert_eq!(p["name_taken"], true);
    assert!(errors(&p).contains(&"Tên LoRA đã có, hãy đặt tên khác".to_string()));
    sqlx::query("UPDATE loras SET status = 'archived' WHERE id = 'l1'")
        .execute(&app.state.db)
        .await
        .unwrap();
    let p = app
        .call("POST", &url, &carol, Some(body.clone()))
        .await
        .json();
    assert_eq!(p["name_taken"], false);
}
