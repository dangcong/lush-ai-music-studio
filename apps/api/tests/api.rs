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
        json!([{"engine": "acestep", "key": "acestep-v15-xl-sft", "name": "ACE-Step 1.5 XL SFT"}]);
    let (_, token_a) = app.node(&admin, "render-node-01", mock_models()).await;
    let (id_b, token_b) = app.node(&admin, "render-node-02", other.clone()).await;
    let mock_id = app.model_id(&alice, "mock-v1").await;
    let ace_id = app.model_id(&alice, "acestep-v15-xl-sft").await;

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
        {"engine": "acestep", "key": "ace-xl", "name": "ACE XL", "capabilities": ["generate", "lora", "reference_audio", "cover"]},
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
    let ace = app.model_id(&alice, "ace-xl").await;
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
            Some(json!({"lease_token": lease, "result": {"seed": 7, "duration": 30.0, "sample_rate": 48000, "engine": "acestep", "model_key": "ace-xl", "engine_info": {"task_type": "text2music"}}})),
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
            "models": [{"engine": "acestep", "key": "acestep-v15-xl-sft", "capabilities": ["generate", "lora"]}],
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
    let model_id = app.model_id(&admin, "acestep-v15-xl-sft").await;

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
    assert!(detail["snapshot"].is_null());
    assert_eq!(detail["checkpoints"], json!([]));

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
        {"engine": "acestep", "key": "acestep-v15-xl-sft", "capabilities": ["generate"]},
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
    assert_eq!(kind_of("acestep-v15-xl-sft"), "generator");
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

/// Upload one track to a fresh lo-fi catalog (60–95 BPM) and complete its analysis with `bpm`.
async fn analyzed_tempo(app: &TestApp, admin: &str, cookie: &str, bpm: f64) -> Value {
    let (_, token) = app.analysis_node(admin, &format!("fold-{bpm}")).await;
    let catalog = lofi_catalog(app, cookie, &format!("Fold {bpm}")).await;
    app.upload_track(cookie, &catalog, "f.wav", WAV).await;
    let job = app.claim(&token).await.json();
    app.call(
        "POST",
        &format!(
            "/api/worker/v1/jobs/{}/complete",
            job["id"].as_str().unwrap()
        ),
        &token,
        Some(json!({"lease_token": job["lease_token"], "result": analysis_result(bpm, 100.0)})),
    )
    .await;
    let list = app
        .call(
            "GET",
            &format!("/api/v1/catalogs/{catalog}/tracks"),
            cookie,
            None,
        )
        .await
        .json();
    list[0].clone()
}

#[tokio::test]
async fn analyze_folds_tempo_into_the_genre_range() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    // librosa often doubles lo-fi: 152 -> 76 (the half is inside 60–95).
    let t = analyzed_tempo(&app, &admin, &carol, 152.0).await;
    assert_eq!(t["tempo_bpm"].as_f64(), Some(76.0));
    assert_eq!(t["tempo_source"], "auto");
    assert_eq!(t["estimates"]["tempo"]["bpm"].as_f64(), Some(152.0));
    assert_eq!(t["estimates"]["tempo"]["genre_bpm"].as_f64(), Some(76.0));
    // Half tempo: 40 -> 80.
    let t = analyzed_tempo(&app, &admin, &carol, 40.0).await;
    assert_eq!(t["tempo_bpm"].as_f64(), Some(80.0));
    // Inside the range: unchanged, no genre_bpm.
    let t = analyzed_tempo(&app, &admin, &carol, 78.0).await;
    assert_eq!(t["tempo_bpm"].as_f64(), Some(78.0));
    assert!(t["estimates"]["tempo"]["genre_bpm"].is_null());
    // Neither the half nor the double fits: the measurement is kept.
    let t = analyzed_tempo(&app, &admin, &carol, 300.0).await;
    assert_eq!(t["tempo_bpm"].as_f64(), Some(300.0));
    assert!(t["estimates"]["tempo"]["genre_bpm"].is_null());
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
async fn analyze_does_not_fold_a_human_tempo() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    let (_, token) = app.analysis_node(&admin, "analysis-node").await;
    let catalog = lofi_catalog(&app, &carol, "Human fold").await;
    let track = app
        .upload_track(&carol, &catalog, "h.wav", WAV)
        .await
        .json();
    sqlx::query("UPDATE catalog_tracks SET tempo_bpm = 140.0, tempo_source = 'human' WHERE id = ?")
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
        Some(json!({"lease_token": job["lease_token"], "result": analysis_result(152.0, 100.0)})),
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
    assert_eq!(list[0]["tempo_bpm"].as_f64(), Some(140.0));
    assert_eq!(list[0]["tempo_source"], "human");
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
    // Hand-typed shorthand is stored in the keyscale form ACE-Step was trained on.
    let t = app
        .call("PATCH", &url, &carol, Some(json!({"key_name": "c#m"})))
        .await
        .json();
    assert_eq!(t["key_name"], "C# minor");
    let t = app
        .call("PATCH", &url, &carol, Some(json!({"key_name": "A minor"})))
        .await
        .json();
    assert_eq!(t["key_name"], "A minor");
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
        json!({"key_name": "A dorian"}),
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
async fn lora_preview_flags_unreviewed_and_suspect_captions() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    captioner_node(&app, &admin).await;
    let model_id = app.model_id(&admin, "mock-v1").await;
    let cat = lofi_catalog(&app, &carol, "Review").await;
    let ids = three_tracks(&app, &carol, &cat).await;
    for (track, text) in [
        (&ids[0], "warm rhodes, dusty drums"),
        (&ids[1], "mellow piano with a Cinematic brass section"),
        (&ids[2], "soft guitar loop"),
    ] {
        put_caption(&app, &carol, track, "cap-x", caption_body(text)).await;
        app.call(
            "PATCH",
            &format!("/api/v1/catalog-tracks/{track}"),
            &carol,
            Some(json!({"current_model_key": "cap-x"})),
        )
        .await;
    }
    // Tracks 0 and 1 keep the machine's caption (nobody reviewed it).
    for track in [&ids[0], &ids[1]] {
        sqlx::query("UPDATE caption_revisions SET source = 'model' WHERE catalog_track_id = ?")
            .bind(track)
            .execute(&app.state.db)
            .await
            .unwrap();
    }
    let url = format!("/api/v1/catalogs/{cat}/lora-preview");
    let mut body = lora_build_body(&model_id, &[&ids[0], &ids[1], &ids[2]]);
    body["holdout_count"] = json!(0);
    let p = app
        .call("POST", &url, &carol, Some(body.clone()))
        .await
        .json();
    // Warnings only: the run can still start.
    assert_eq!(p["ready"], true, "{p}");
    let unreviewed: Vec<&str> = p["unreviewed_caption"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert_eq!(unreviewed.len(), 2);
    assert!(unreviewed.contains(&ids[0].as_str()) && unreviewed.contains(&ids[1].as_str()));
    let warnings = p["warnings"].as_array().unwrap();
    assert!(
        warnings
            .iter()
            .any(|w| w == "2 bài dùng mô tả máy chưa duyệt"),
        "{p}"
    );
    assert!(
        warnings
            .iter()
            .any(|w| w == "1 bài có từ lạ với genre: brass, cinematic"),
        "{p}"
    );
    let suspect = p["train"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == ids[1].as_str())
        .unwrap();
    assert_eq!(suspect["suspect_words"], json!(["brass", "cinematic"]));
    assert_eq!(suspect["caption_source"], "model");

    // "Duyệt mô tả" = saving the same text as a human revision.
    put_caption(
        &app,
        &carol,
        &ids[0],
        "cap-x",
        caption_body("warm rhodes, dusty drums"),
    )
    .await;
    let p = app.call("POST", &url, &carol, Some(body)).await.json();
    assert_eq!(p["unreviewed_caption"].as_array().unwrap().len(), 1);
}

/// `n` captioned tracks in a fresh lo-fi catalog, each with a `style_embedding` from `embedding(i)`
/// (`None` = analysed without MERT).
async fn styled_tracks(
    app: &TestApp,
    cookie: &str,
    name: &str,
    n: u8,
    embedding: impl Fn(u8) -> Option<Value>,
) -> (String, Vec<String>) {
    let cat = lofi_catalog(app, cookie, name).await;
    let mut ids = vec![];
    for i in 0..n {
        let mut data = WAV.to_vec();
        data.push(b'a' + i);
        let id = app
            .upload_track(cookie, &cat, &format!("s{i}.wav"), &data)
            .await
            .json()["id"]
            .as_str()
            .unwrap()
            .to_string();
        put_caption(app, cookie, &id, "cap-x", caption_body("warm rhodes")).await;
        app.call(
            "PATCH",
            &format!("/api/v1/catalog-tracks/{id}"),
            cookie,
            Some(json!({"current_model_key": "cap-x"})),
        )
        .await;
        let mut analysis = analysis_result(80.0, 100.0);
        if let Some(e) = embedding(i) {
            analysis["style_embedding"] = e;
        }
        sqlx::query("UPDATE catalog_tracks SET analysis = ? WHERE id = ?")
            .bind(analysis.to_string())
            .bind(&id)
            .execute(&app.state.db)
            .await
            .unwrap();
        ids.push(id);
    }
    (cat, ids)
}

#[tokio::test]
async fn lora_preview_flags_style_outliers() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    captioner_node(&app, &admin).await;
    let model_id = app.model_id(&admin, "mock-v1").await;
    // Six songs close to one direction, one (index 6) pointing elsewhere.
    let (cat, ids) = styled_tracks(&app, &carol, "Style", 7, |i| {
        let j = f64::from(i) * 0.01;
        Some(if i == 6 {
            json!([0.1, 1.0, 0.2])
        } else {
            json!([1.0, 0.05 + j, 0.1 - j])
        })
    })
    .await;
    let refs: Vec<&String> = ids.iter().collect();
    let mut body = lora_build_body(&model_id, &refs);
    body["holdout_count"] = json!(0);
    let url = format!("/api/v1/catalogs/{cat}/lora-preview");
    let p = app.call("POST", &url, &carol, Some(body)).await.json();
    assert_eq!(p["ready"], true, "{p}");
    let outliers = p["style_outliers"].as_array().unwrap();
    assert_eq!(outliers.len(), 1, "{p}");
    assert_eq!(outliers[0]["id"], ids[6].as_str());
    assert_eq!(outliers[0]["filename"], "s6.wav");
    assert!(outliers[0]["score"].as_f64().unwrap() < 0.5);
    assert_eq!(p["style_unmeasured"], 0);
    assert!(
        p["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w == "1 bài lạc chất so với nhóm: s6.wav"),
        "{p}"
    );
}

#[tokio::test]
async fn lora_preview_style_check_needs_four_measured_songs() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    captioner_node(&app, &admin).await;
    let model_id = app.model_id(&admin, "mock-v1").await;
    // Three measured (one of them odd) and two analysed before MERT.
    let (cat, ids) = styled_tracks(&app, &carol, "Few", 5, |i| match i {
        0 | 1 => Some(json!([1.0, 0.0, 0.0])),
        2 => Some(json!([0.0, 1.0, 0.0])),
        _ => None,
    })
    .await;
    let refs: Vec<&String> = ids.iter().collect();
    let mut body = lora_build_body(&model_id, &refs);
    body["holdout_count"] = json!(0);
    let url = format!("/api/v1/catalogs/{cat}/lora-preview");
    let p = app.call("POST", &url, &carol, Some(body)).await.json();
    assert_eq!(p["style_outliers"], json!([]));
    assert_eq!(p["style_unmeasured"], 2);
    assert!(
        p["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w == "2 bài chưa đo độ hợp nhóm (phân tích lại để đo)"),
        "{p}"
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

// ---------- LoRA create from catalog ----------

struct LoraSetup {
    cat: String,
    ids: Vec<String>,
    model_id: String,
    token: String,
}

/// A catalog of three captioned songs and a node that trains but holds no dataset.
async fn lora_setup(app: &TestApp, admin: &str, carol: &str) -> LoraSetup {
    captioner_node(app, admin).await;
    let created = app
        .call(
            "POST",
            "/api/v1/nodes",
            admin,
            Some(json!({"name": "lora-train"})),
        )
        .await;
    let token = created.json()["token"].as_str().unwrap().to_string();
    let reg = json!({
        "protocol_version": 1, "worker_version": "test", "fresh_start": true,
        "job_types": ["generate", "train_lora"],
        "engines": [{"name": "acestep", "version": "1", "capabilities": ["generate", "lora"]}],
        "models": [{"engine": "acestep", "key": "acestep-v15-xl-sft", "capabilities": ["generate", "lora"]}],
        "loras": [], "datasets": [], "gpu": null,
    });
    let r = app
        .call("POST", "/api/worker/v1/register", &token, Some(reg))
        .await;
    assert_eq!(r.status, StatusCode::OK);
    let model_id = app.model_id(admin, "acestep-v15-xl-sft").await;
    let cat = lofi_catalog(app, carol, "Lora create").await;
    let ids = three_tracks(app, carol, &cat).await;
    for t in &ids {
        let r = put_caption(app, carol, t, "cap-x", caption_body("warm rhodes")).await;
        assert_eq!(r.status, StatusCode::OK);
        let r = app
            .call(
                "PATCH",
                &format!("/api/v1/catalog-tracks/{t}"),
                carol,
                Some(json!({"current_model_key": "cap-x"})),
            )
            .await;
        assert_eq!(r.status, StatusCode::OK);
    }
    LoraSetup {
        cat,
        ids,
        model_id,
        token,
    }
}

fn lora_create_body(s: &LoraSetup) -> Value {
    let mut b = lora_build_body(&s.model_id, &[&s.ids[0], &s.ids[1], &s.ids[2]]);
    b["keyword"] = json!("lushlofi");
    b
}

#[tokio::test]
async fn create_lora_from_catalog_snapshots_and_queues() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    let s = lora_setup(&app, &admin, &carol).await;
    let body = lora_create_body(&s);
    let r = app
        .call(
            "POST",
            &format!("/api/v1/catalogs/{}/loras", s.cat),
            &carol,
            Some(body),
        )
        .await;
    assert_eq!(r.status, StatusCode::CREATED);
    let created = r.json();
    let snap = app
        .call(
            "GET",
            &format!(
                "/api/v1/dataset-snapshots/{}",
                created["snapshot_id"].as_str().unwrap()
            ),
            &carol,
            None,
        )
        .await;
    assert_eq!(snap.status, StatusCode::OK);
    let snap = snap.json();
    assert_eq!(snap["job_id"], created["job_id"]);
    let c = &snap["content"];
    assert_eq!(c["samples"].as_array().unwrap().len(), 2);
    assert_eq!(c["held_out"].as_array().unwrap().len(), 1);
    // keyword first, then the caption, then the Instruments / Mood items it does not mention.
    assert_eq!(
        c["samples"][0]["train_caption"],
        "lushlofi, warm rhodes, piano, calm"
    );
    assert_eq!(c["config"]["max_duration_s"], 240);
    assert_eq!(c["lora"]["name"], "Lo-fi Studio v1");

    let job = app.claim(&s.token).await.json();
    assert_eq!(job["type"], "train_lora");
    assert_eq!(
        job["payload"]["dataset"]["snapshot_id"],
        created["snapshot_id"]
    );
    assert_eq!(job["payload"]["dataset"]["key"], created["lora_key"]);
    assert_eq!(job["payload"]["samples"].as_array().unwrap().len(), 2);
    let art = job["payload"]["inputs"]["t0"]["artifact_id"]
        .as_str()
        .unwrap();
    let inp = app
        .call(
            "GET",
            &format!(
                "/api/worker/v1/jobs/{}/inputs/{art}?lease_token={}",
                job["id"].as_str().unwrap(),
                job["lease_token"].as_str().unwrap()
            ),
            &s.token,
            None,
        )
        .await;
    assert_eq!(inp.status, StatusCode::OK);
}

#[tokio::test]
async fn lora_preview_warns_when_a_train_caption_is_cut() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    let s = lora_setup(&app, &admin, &carol).await;
    let url = format!("/api/v1/catalogs/{}/lora-preview", s.cat);
    let warned = |p: &Value| {
        p["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap().contains("phần cuối sẽ bị bỏ khi train"))
    };
    let plan = app
        .call("POST", &url, &carol, Some(lora_create_body(&s)))
        .await
        .json();
    assert!(!warned(&plan));
    let long = "A slow warm lo-fi groove with dusty drums. ".repeat(30);
    for t in &s.ids {
        let r = put_caption(&app, &carol, t, "cap-x", caption_body(&long)).await;
        assert_eq!(r.status, StatusCode::OK);
    }
    let plan = app
        .call("POST", &url, &carol, Some(lora_create_body(&s)))
        .await
        .json();
    assert!(warned(&plan), "{plan}");
}

#[tokio::test]
async fn lora_snapshot_is_immutable_and_independent_of_catalog_edits() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    let s = lora_setup(&app, &admin, &carol).await;
    let r = app
        .call(
            "POST",
            &format!("/api/v1/catalogs/{}/loras", s.cat),
            &carol,
            Some(lora_create_body(&s)),
        )
        .await;
    assert_eq!(r.status, StatusCode::CREATED);
    let sid = r.json()["snapshot_id"].as_str().unwrap().to_string();
    let url = format!("/api/v1/dataset-snapshots/{sid}");
    let before = app.call("GET", &url, &carol, None).await.json();
    let train_track = before["content"]["samples"][0]["catalog_track_id"]
        .as_str()
        .unwrap()
        .to_string();
    let r = put_caption(&app, &carol, &train_track, "cap-x", caption_body("changed")).await;
    assert_eq!(r.status, StatusCode::OK);
    let r = app
        .call(
            "PATCH",
            &format!("/api/v1/catalog-tracks/{train_track}"),
            &carol,
            Some(json!({"tempo_bpm": 99})),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK);
    let after = app.call("GET", &url, &carol, None).await.json();
    assert_eq!(before["content"], after["content"]);
    assert!(
        sqlx::query("UPDATE dataset_snapshots SET content = '{}'")
            .execute(&app.state.db)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn create_lora_rejections() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    let alice = app.login("alice").await;
    let s = lora_setup(&app, &admin, &carol).await;
    let url = format!("/api/v1/catalogs/{}/loras", s.cat);
    let snapshots = || async {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM dataset_snapshots")
            .fetch_one(&app.state.db)
            .await
            .unwrap()
    };
    let msg = |r: &Reply| r.json()["error"]["message"].as_str().unwrap().to_string();

    assert_eq!(
        app.call("POST", &url, &alice, Some(lora_create_body(&s)))
            .await
            .status,
        StatusCode::FORBIDDEN
    );

    // Not ready: first error is returned.
    let mut body = lora_create_body(&s);
    body["name"] = json!("");
    let r = app.call("POST", &url, &carol, Some(body)).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(msg(&r), "Tên LoRA phải có 1–80 ký tự");
    assert_eq!(snapshots().await, 0);

    // Unknown base model.
    let mut body = lora_create_body(&s);
    body["base_model_id"] = json!("nope");
    let r = app.call("POST", &url, &carol, Some(body)).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(msg(&r), "Base model không tồn tại");
    // Model without a trainer.
    let mut body = lora_create_body(&s);
    body["base_model_id"] = json!(app.model_id(&admin, "mock-v1").await);
    let r = app.call("POST", &url, &carol, Some(body)).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(msg(&r), "Chưa có trainer cho engine của model này");
    assert_eq!(snapshots().await, 0);

    // A song deleted between preview and create.
    let body = lora_create_body(&s);
    let r = app
        .call(
            "DELETE",
            &format!("/api/v1/catalog-tracks/{}", s.ids[2]),
            &carol,
            None,
        )
        .await;
    assert!(r.status.is_success());
    let r = app.call("POST", &url, &carol, Some(body)).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(msg(&r), "Có bài không còn trong catalog này");
    assert_eq!(snapshots().await, 0);

    // Second create while the first is queued.
    let mut body = lora_create_body(&s);
    body["track_ids"] = json!([s.ids[0], s.ids[1]]);
    body["holdout_count"] = json!(0);
    let r = app.call("POST", &url, &carol, Some(body.clone())).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", msg(&r));
    // The identical body again (a double click): 409, not "name taken".
    let r = app.call("POST", &url, &carol, Some(body)).await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    assert_eq!(msg(&r), "Đang có một training run khác chưa kết thúc");
    assert_eq!(snapshots().await, 1);
}

#[tokio::test]
async fn create_lora_needs_a_training_node() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    let s = lora_setup(&app, &admin, &carol).await;
    sqlx::query("UPDATE nodes SET job_types = '[\"generate\"]'")
        .execute(&app.state.db)
        .await
        .unwrap();
    let r = app
        .call(
            "POST",
            &format!("/api/v1/catalogs/{}/loras", s.cat),
            &carol,
            Some(lora_create_body(&s)),
        )
        .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        r.json()["error"]["message"],
        "Không có render node nào hỗ trợ training"
    );
}

#[tokio::test]
async fn lora_name_taken_by_live_job_and_unicode_names() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    let s = lora_setup(&app, &admin, &carol).await;
    let url = format!("/api/v1/catalogs/{}/loras", s.cat);
    let preview = format!("/api/v1/catalogs/{}/lora-preview", s.cat);
    let r = app
        .call("POST", &url, &carol, Some(lora_create_body(&s)))
        .await;
    assert_eq!(r.status, StatusCode::CREATED);
    let job_id = r.json()["job_id"].as_str().unwrap().to_string();
    let title: String = sqlx::query_scalar("SELECT title FROM jobs WHERE id = ?")
        .bind(&job_id)
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    assert_eq!(title, "Lo-fi Studio v1");
    let mut body = lora_create_body(&s);
    body["name"] = json!("LO-FI STUDIO V1");
    let p = app
        .call("POST", &preview, &carol, Some(body.clone()))
        .await
        .json();
    assert_eq!(p["name_taken"], true);
    assert!(
        p["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e == "Tên LoRA đã có, hãy đặt tên khác")
    );

    // Unicode-aware comparison against an existing LoRA.
    sqlx::query("UPDATE jobs SET status = 'failed' WHERE id = ?")
        .bind(&job_id)
        .execute(&app.state.db)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO loras (id, key, name, base_model_id, status, created_at) \
         VALUES ('l2', 'dem-lo-fi-aaaa', 'Đêm Lo-fi', ?, 'evaluation', '2026-01-01T00:00:00Z')",
    )
    .bind(&s.model_id)
    .execute(&app.state.db)
    .await
    .unwrap();
    body["name"] = json!("đêm lo-fi");
    let p = app.call("POST", &preview, &carol, Some(body)).await.json();
    assert_eq!(p["name_taken"], true);
}

// ---------- LoRA run completion: checkpoints and snapshot ----------

fn ckpt(epoch: i64, sha: &str) -> Value {
    json!({"epoch": epoch, "step": epoch, "path": format!("/x/epoch_{epoch}"), "sha256": sha, "bytes": 10})
}

#[tokio::test]
async fn snapshot_run_completion_records_checkpoints() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    let s = lora_setup(&app, &admin, &carol).await;
    let created = app
        .call(
            "POST",
            &format!("/api/v1/catalogs/{}/loras", s.cat),
            &carol,
            Some(lora_create_body(&s)),
        )
        .await
        .json();
    let job = app.claim(&s.token).await.json();
    let job_id = job["id"].as_str().unwrap().to_string();
    let sha = "a".repeat(64);
    let r = app
        .call(
            "POST",
            &format!("/api/worker/v1/jobs/{job_id}/complete"),
            &s.token,
            Some(json!({"lease_token": job["lease_token"], "result": {
                "lora": {"key": created["lora_key"], "adapter_sha256": sha},
                "checkpoints": [ckpt(1, &sha), ckpt(2, &sha)],
                "run_dir": "/node/training/secret-run-dir",
            }})),
        )
        .await;
    assert_eq!(
        r.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&r.bytes)
    );

    let r = app
        .call(
            "GET",
            &format!("/api/v1/training-runs/{job_id}"),
            &carol,
            None,
        )
        .await;
    assert_eq!(r.status, StatusCode::OK);
    let body = String::from_utf8_lossy(&r.bytes).to_string();
    assert!(
        !body.contains("node_path")
            && !body.contains("/x/epoch_1")
            && !body.contains("run_dir")
            && !body.contains("secret-run-dir"),
        "{body}"
    );
    let d = r.json();
    let cps = d["checkpoints"].as_array().unwrap();
    assert_eq!(cps.len(), 2);
    assert_eq!(cps[0]["epoch"], 1);
    assert_eq!(cps[1]["epoch"], 2);
    assert!(cps.iter().all(|c| c["status"] == "kept"));
    assert_eq!(d["snapshot"]["id"], created["snapshot_id"]);
    assert_eq!(d["snapshot"]["samples"], 2);
    assert_eq!(d["snapshot"]["held_out"].as_array().unwrap().len(), 1);
    assert_eq!(d["snapshot"]["keyword"], "lushlofi");
    assert_eq!(d["snapshot"]["catalog_id"], s.cat.as_str());
    assert!(d["snapshot"]["models"].is_array());
    assert_eq!(d["lora"]["status"], "evaluation");
    let list = app.call("GET", "/api/v1/training-runs", &carol, None).await;
    assert_eq!(list.status, StatusCode::OK);

    let (snap, status): (Option<String>, String) =
        sqlx::query_as("SELECT snapshot_id, status FROM loras WHERE job_id = ?")
            .bind(&job_id)
            .fetch_one(&app.state.db)
            .await
            .unwrap();
    assert_eq!(snap.as_deref(), created["snapshot_id"].as_str());
    assert_eq!(status, "evaluation");
    let dk: Option<String> = sqlx::query_scalar("SELECT dataset_key FROM loras WHERE job_id = ?")
        .bind(&job_id)
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    assert_eq!(dk.as_deref(), created["lora_key"].as_str());
}

#[tokio::test]
async fn snapshot_run_completion_validates_checkpoints() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    let s = lora_setup(&app, &admin, &carol).await;
    let created = app
        .call(
            "POST",
            &format!("/api/v1/catalogs/{}/loras", s.cat),
            &carol,
            Some(lora_create_body(&s)),
        )
        .await
        .json();
    let job = app.claim(&s.token).await.json();
    let job_id = job["id"].as_str().unwrap().to_string();
    let url = format!("/api/worker/v1/jobs/{job_id}/complete");
    let sha = "b".repeat(64);
    let lora = json!({"key": created["lora_key"], "adapter_sha256": sha});
    let send = |result: Value| {
        app.call(
            "POST",
            &url,
            &s.token,
            Some(json!({"lease_token": job["lease_token"], "result": result})),
        )
    };

    let r = send(json!({"lora": lora, "checkpoints": []})).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(String::from_utf8_lossy(&r.bytes).contains("Job train_lora chưa báo checkpoint"));
    let r = send(json!({"lora": lora})).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    for bad in [
        ckpt(1, "xyz"),
        ckpt(1, &"A".repeat(64)),
        ckpt(0, &sha),
        json!({"epoch": 1, "path": "", "sha256": sha}),
    ] {
        let r = send(json!({"lora": lora, "checkpoints": [bad]})).await;
        assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(String::from_utf8_lossy(&r.bytes).contains("checkpoint không hợp lệ"));
    }
    assert_eq!(app.job(&carol, &job_id).await["status"], "preparing");
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM loras")
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    assert_eq!(n, 0);

    let r = send(json!({"lora": lora, "checkpoints": [ckpt(1, &sha)]})).await;
    assert_eq!(r.status, StatusCode::OK);
    let again = send(json!({"lora": lora, "checkpoints": [ckpt(1, &sha)]})).await;
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(again.json()["ok"], true);
}

// ---------- LoRA review: eval_lora queue, node pinning ----------

/// Claim and complete the snapshot run with checkpoints for `epochs`. Returns (run job id, node id).
async fn finish_snapshot_run(
    app: &TestApp,
    s: &LoraSetup,
    created: &Value,
    epochs: &[i64],
) -> (String, String) {
    let job = app.claim(&s.token).await.json();
    let job_id = job["id"].as_str().unwrap().to_string();
    let sha = "a".repeat(64);
    let cps: Vec<Value> = epochs.iter().map(|e| ckpt(*e, &sha)).collect();
    let r = app
        .call(
            "POST",
            &format!("/api/worker/v1/jobs/{job_id}/complete"),
            &s.token,
            Some(json!({"lease_token": job["lease_token"], "result": {
                "lora": {"key": created["lora_key"], "adapter_sha256": sha},
                "checkpoints": cps,
            }})),
        )
        .await;
    assert_eq!(
        r.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&r.bytes)
    );
    let node_id: String = sqlx::query_scalar("SELECT node_id FROM jobs WHERE id = ?")
        .bind(&job_id)
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    (job_id, node_id)
}

async fn start_snapshot_run(app: &TestApp, carol: &str, s: &LoraSetup) -> Value {
    let r = app
        .call(
            "POST",
            &format!("/api/v1/catalogs/{}/loras", s.cat),
            carol,
            Some(lora_create_body(s)),
        )
        .await;
    assert_eq!(r.status, StatusCode::CREATED);
    r.json()
}

/// (status, priority, payload) of every eval_lora job.
async fn eval_jobs(app: &TestApp) -> Vec<(String, i64, Value)> {
    let rows: Vec<(String, i64, String)> = sqlx::query_as(
        "SELECT status, priority, payload FROM jobs WHERE type = 'eval_lora' ORDER BY created_at",
    )
    .fetch_all(&app.state.db)
    .await
    .unwrap();
    rows.into_iter()
        .map(|(s, p, j)| (s, p, serde_json::from_str(&j).unwrap()))
        .collect()
}

async fn set_lofi_duration(app: &TestApp, min: Option<i64>, max: Option<i64>) {
    sqlx::query("UPDATE genres SET duration_min = ?, duration_max = ? WHERE slug = 'lo-fi'")
        .bind(min)
        .bind(max)
        .execute(&app.state.db)
        .await
        .unwrap();
}

#[tokio::test]
async fn eval_job_is_queued_when_snapshot_run_completes() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    let s = lora_setup(&app, &admin, &carol).await;
    set_lofi_duration(&app, Some(30), Some(120)).await;
    let created = start_snapshot_run(&app, &carol, &s).await;
    assert!(eval_jobs(&app).await.is_empty());
    let (run_id, node_id) = finish_snapshot_run(&app, &s, &created, &[1, 2]).await;

    let jobs = eval_jobs(&app).await;
    assert_eq!(jobs.len(), 1);
    let (status, priority, p) = &jobs[0];
    assert_eq!(status, "queued");
    assert_eq!(*priority, -30);
    assert_eq!(p["node_id"], node_id.as_str());
    let key = created["lora_key"].as_str().unwrap();
    assert_eq!(p["dataset_key"], key);
    assert_eq!(p["lora"]["base_key"], key);
    assert_eq!(
        p["checkpoints"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["key"].as_str().unwrap().to_string())
            .collect::<Vec<_>>(),
        vec![format!("{key}-e1"), format!("{key}-e2")]
    );
    assert_eq!(p["checkpoints"][0]["epoch"], 1);
    assert_eq!(p["checkpoints"][1]["path"], "/x/epoch_2");
    assert_eq!(p["checkpoints"][0]["sha256"], "a".repeat(64));
    let lora_id: String = sqlx::query_scalar("SELECT id FROM loras WHERE job_id = ?")
        .bind(&run_id)
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    assert_eq!(p["lora"]["id"], lora_id.as_str());
    assert_eq!(p["duration"], 60);
    assert_eq!(p["scale"], 0.7);
    assert_eq!(p["seed"], 42);
    // Prompts are final: the worker uses them verbatim, composed like any generate request.
    let template: String =
        sqlx::query_scalar("SELECT prompt_template FROM genres WHERE slug = 'lo-fi'")
            .fetch_one(&app.state.db)
            .await
            .unwrap();
    let compose =
        |prompt: &str| lush_api::routes::jobs::compose_prompt(prompt, &template, &[], &[], true);
    assert_eq!(p["test_prompt"], compose("warm lo-fi beat").as_str());
    assert_eq!(p["render_template"]["prompt"], p["test_prompt"]);

    let snap = app
        .call(
            "GET",
            &format!(
                "/api/v1/dataset-snapshots/{}",
                created["snapshot_id"].as_str().unwrap()
            ),
            &carol,
            None,
        )
        .await
        .json();
    let c = &snap["content"];
    assert_eq!(p["held_out"].as_array().unwrap().len(), 1);
    let caption = c["held_out"][0]["train_caption"].as_str().unwrap();
    assert!(caption.starts_with("lushlofi, "));
    assert_eq!(p["held_out"][0]["prompt"], compose(caption).as_str());
    assert!(
        p["held_out"][0]["prompt"]
            .as_str()
            .unwrap()
            .contains(caption)
    );
    assert_eq!(p["held_out"][0]["filename"], c["held_out"][0]["filename"]);
    assert_eq!(p["train"].as_array().unwrap().len(), 2);
    assert_eq!(p["train"][0]["dataset_file"], "s000.wav");
    assert_eq!(p["train"][1]["dataset_file"], "s001.wav");
    assert_eq!(p["train"][1]["filename"], c["samples"][1]["filename"]);

    // The template is a generate payload for the base model, seed from the snapshot, no LoRA.
    let t = &p["render_template"];
    assert_eq!(t["duration"], 60);
    assert_eq!(t["seed"], 42);
    assert_eq!(t["model"]["key"], "acestep-v15-xl-sft");
    assert!(t["lora"].is_null());
    assert!(t["prompt"].as_str().unwrap().contains("warm lo-fi beat"));

    // Owned by whoever created the snapshot; created inside the completion transaction.
    let owner: String = sqlx::query_scalar("SELECT owner_id FROM jobs WHERE type = 'eval_lora'")
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    let creator: String = sqlx::query_scalar("SELECT created_by FROM dataset_snapshots")
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    assert_eq!(owner, creator);
}

#[tokio::test]
async fn eval_duration_is_clamped_into_the_genre_range() {
    for (min, max, want) in [
        (Some(90), Some(120), 90),
        (Some(10), Some(40), 40),
        (None, None, 60),
    ] {
        let app = TestApp::new().await;
        let admin = app.login("admin").await;
        let carol = app.login("carol").await;
        let s = lora_setup(&app, &admin, &carol).await;
        set_lofi_duration(&app, min, max).await;
        let created = start_snapshot_run(&app, &carol, &s).await;
        finish_snapshot_run(&app, &s, &created, &[1]).await;
        let jobs = eval_jobs(&app).await;
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].2["duration"], want, "{min:?}-{max:?}");
        assert_eq!(jobs[0].2["render_template"]["duration"], want);
    }
}

#[tokio::test]
async fn no_eval_job_for_legacy_or_failed_runs() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    let s = lora_setup(&app, &admin, &carol).await;

    // A run without a snapshot (old style, dataset already on the node) completes: no eval.
    let reg = json!({
        "protocol_version": 1, "worker_version": "test", "fresh_start": true,
        "job_types": ["generate", "train_lora"],
        "engines": [{"name": "acestep", "version": "1", "capabilities": ["generate", "lora"]}],
        "models": [{"engine": "acestep", "key": "acestep-v15-xl-sft", "capabilities": ["generate", "lora"]}],
        "loras": [], "datasets": [{"key": "jazz/v1", "samples": 8, "seconds": 480}], "gpu": null,
    });
    let r = app
        .call("POST", "/api/worker/v1/register", &s.token, Some(reg))
        .await;
    assert_eq!(r.status, StatusCode::OK);
    let r = app
        .call(
            "POST",
            "/api/v1/training-runs",
            &admin,
            Some(training_body(&s.model_id)),
        )
        .await;
    assert_eq!(r.status, StatusCode::CREATED);
    let job = app.claim(&s.token).await.json();
    let r = app
        .call(
            "POST",
            &format!(
                "/api/worker/v1/jobs/{}/complete",
                job["id"].as_str().unwrap()
            ),
            &s.token,
            Some(json!({"lease_token": job["lease_token"], "result": {
                "lora": {"key": "jazz-v1", "adapter_sha256": "c".repeat(64)}}})),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(eval_jobs(&app).await.is_empty());

    // A snapshot run that fails, and one that is cancelled while queued: no eval.
    start_snapshot_run(&app, &carol, &s).await;
    let job = app.claim(&s.token).await.json();
    let r = app
        .call(
            "POST",
            &format!("/api/worker/v1/jobs/{}/fail", job["id"].as_str().unwrap()),
            &s.token,
            Some(json!({"lease_token": job["lease_token"], "error": "oom", "retryable": false})),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(eval_jobs(&app).await.is_empty());
    let again = start_snapshot_run(&app, &carol, &s).await;
    let r = app
        .call(
            "POST",
            &format!("/api/v1/jobs/{}/cancel", again["job_id"].as_str().unwrap()),
            &carol,
            None,
        )
        .await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(eval_jobs(&app).await.is_empty());
}

#[tokio::test]
async fn eval_payload_keeps_last_ten_checkpoints() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    let s = lora_setup(&app, &admin, &carol).await;
    let created = start_snapshot_run(&app, &carol, &s).await;
    let epochs: Vec<i64> = (1..=12).collect();
    finish_snapshot_run(&app, &s, &created, &epochs).await;
    let jobs = eval_jobs(&app).await;
    let cps = jobs[0].2["checkpoints"].as_array().unwrap();
    assert_eq!(cps.len(), 10);
    let got: Vec<i64> = cps.iter().map(|c| c["epoch"].as_i64().unwrap()).collect();
    assert_eq!(got, (3..=12).collect::<Vec<_>>());
}

#[tokio::test]
async fn pinned_job_is_claimed_only_by_its_node() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let mut nodes = vec![];
    for name in ["node-a", "node-b"] {
        let created = app
            .call("POST", "/api/v1/nodes", &admin, Some(json!({"name": name})))
            .await;
        let token = created.json()["token"].as_str().unwrap().to_string();
        let reg = json!({
            "protocol_version": 1, "worker_version": "test", "fresh_start": true,
            "job_types": ["eval_lora"], "engines": [], "models": [], "loras": [], "gpu": null,
        });
        let r = app
            .call("POST", "/api/worker/v1/register", &token, Some(reg))
            .await;
        assert_eq!(r.status, StatusCode::OK);
        nodes.push((r.json()["node_id"].as_str().unwrap().to_string(), token));
    }
    let owner: String = sqlx::query_scalar("SELECT id FROM users WHERE username = 'admin'")
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    // Pinned to A with the higher priority, an unpinned job below it.
    for (id, prio, payload) in [
        ("pin1", -20, json!({"node_id": nodes[0].0})),
        ("free1", -30, json!({})),
    ] {
        sqlx::query(
            "INSERT INTO jobs (id, type, priority, owner_id, title, payload, created_at, updated_at)              VALUES (?, 'eval_lora', ?, ?, 't', ?, 't', 't')",
        )
        .bind(id)
        .bind(prio)
        .bind(&owner)
        .bind(payload.to_string())
        .execute(&app.state.db)
        .await
        .unwrap();
    }
    // B skips the pinned job and takes the unpinned one; the pinned job stays queued.
    let r = app.claim(&nodes[1].1).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.json()["id"], "free1");
    let status: String = sqlx::query_scalar("SELECT status FROM jobs WHERE id = 'pin1'")
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    assert_eq!(status, "queued");
    assert_eq!(app.claim(&nodes[1].1).await.status, StatusCode::NO_CONTENT);
    // A takes its own job.
    let r = app.claim(&nodes[0].1).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.json()["id"], "pin1");
}

#[tokio::test]
async fn new_train_jobs_have_priority_minus_40() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    let s = lora_setup(&app, &admin, &carol).await;
    let created = start_snapshot_run(&app, &carol, &s).await;
    let prio: i64 = sqlx::query_scalar("SELECT priority FROM jobs WHERE id = ?")
        .bind(created["job_id"].as_str().unwrap())
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    assert_eq!(prio, -40);

    // eval_renders = min(saves, 10) * (1 + held_out) + 1: 12 saves, 2 held out.
    let mut body = lora_create_body(&s);
    body["name"] = json!("Another");
    body["epochs"] = json!(12);
    body["save_every"] = json!(1);
    body["holdout_count"] = json!(2);
    let p = app
        .call(
            "POST",
            &format!("/api/v1/catalogs/{}/lora-preview", s.cat),
            &carol,
            Some(body),
        )
        .await
        .json();
    assert_eq!(p["eval_renders"], 10 * 3 + 1);
}

#[tokio::test]
async fn lora_seed_out_of_range_is_rejected_up_front() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    let s = lora_setup(&app, &admin, &carol).await;
    let mut body = lora_create_body(&s);
    body["seed"] = json!(2147483648_i64);
    for path in ["lora-preview", "loras"] {
        let r = app
            .call(
                "POST",
                &format!("/api/v1/catalogs/{}/{path}", s.cat),
                &carol,
                Some(body.clone()),
            )
            .await;
        assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{path}");
    }
    body["seed"] = json!(-1);
    let r = app
        .call(
            "POST",
            &format!("/api/v1/catalogs/{}/loras", s.cat),
            &carol,
            Some(body.clone()),
        )
        .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    body["seed"] = json!(2147483647_i64);
    let r = app
        .call(
            "POST",
            &format!("/api/v1/catalogs/{}/loras", s.cat),
            &carol,
            Some(body),
        )
        .await;
    assert_eq!(r.status, StatusCode::CREATED);
}

#[tokio::test]
async fn long_held_out_caption_is_truncated_for_the_eval_prompt() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    let s = lora_setup(&app, &admin, &carol).await;
    let long = "warm ".repeat(600); // 3000 chars
    for t in &s.ids {
        let r = put_caption(&app, &carol, t, "cap-x", caption_body(&long)).await;
        assert_eq!(r.status, StatusCode::OK);
    }
    let created = start_snapshot_run(&app, &carol, &s).await;
    finish_snapshot_run(&app, &s, &created, &[1]).await;
    let jobs = eval_jobs(&app).await;
    assert_eq!(jobs.len(), 1, "eval job must still be queued");
    let prompt = jobs[0].2["held_out"][0]["prompt"].as_str().unwrap();
    assert!(prompt.chars().count() <= 2000, "{}", prompt.chars().count());
    assert!(
        prompt.starts_with("lushlofi, warm warm"),
        "{}",
        &prompt[..120.min(prompt.len())]
    );
}

// ---------- LoRA review: eval_lora completion, eval_NN artifacts, review fields ----------

struct EvalRun {
    run_id: String,
    lora_id: String,
    job_id: String,
    lease: String,
    /// Checkpoint ids by epoch order.
    cps: Vec<String>,
}

/// Re-register the lora node so that it also takes `eval_lora` jobs.
async fn register_eval_node(app: &TestApp, s: &LoraSetup) {
    let reg = json!({
        "protocol_version": 1, "worker_version": "test", "fresh_start": true,
        "job_types": ["generate", "train_lora", "eval_lora"],
        "engines": [{"name": "acestep", "version": "1", "capabilities": ["generate", "lora"]}],
        "models": [{"engine": "acestep", "key": "acestep-v15-xl-sft", "capabilities": ["generate", "lora"]}],
        "loras": [], "datasets": [], "gpu": null,
    });
    let r = app
        .call("POST", "/api/worker/v1/register", &s.token, Some(reg))
        .await;
    assert_eq!(r.status, StatusCode::OK);
}

/// Finish a snapshot run with `epochs`, let the node claim the queued `eval_lora` job.
async fn claim_eval(app: &TestApp, s: &LoraSetup, created: &Value, epochs: &[i64]) -> EvalRun {
    let (run_id, _) = finish_snapshot_run(app, s, created, epochs).await;
    register_eval_node(app, s).await;
    let lora_id: String = sqlx::query_scalar("SELECT id FROM loras WHERE job_id = ?")
        .bind(&run_id)
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    let cps: Vec<String> =
        sqlx::query_scalar("SELECT id FROM lora_checkpoints WHERE lora_id = ? ORDER BY epoch")
            .bind(&lora_id)
            .fetch_all(&app.state.db)
            .await
            .unwrap();
    let job = app.claim(&s.token).await;
    assert_eq!(job.status, StatusCode::OK);
    let job = job.json();
    assert_eq!(job["type"], "eval_lora");
    EvalRun {
        run_id,
        lora_id,
        job_id: job["id"].as_str().unwrap().to_string(),
        lease: job["lease_token"].as_str().unwrap().to_string(),
        cps,
    }
}

async fn upload_eval(app: &TestApp, s: &LoraSetup, e: &EvalRun, kind: &str, file: &str) -> Reply {
    let data = format!("fake mp3 {kind}");
    app.upload(
        &s.token,
        &e.job_id,
        &e.lease,
        kind,
        file,
        data.as_bytes(),
        &sha256_hex(data.as_bytes()),
    )
    .await
}

async fn complete_eval_job(app: &TestApp, s: &LoraSetup, e: &EvalRun, result: Value) -> Reply {
    app.call(
        "POST",
        &format!("/api/worker/v1/jobs/{}/complete", e.job_id),
        &s.token,
        Some(json!({"lease_token": e.lease, "result": result})),
    )
    .await
}

fn eval_render(
    kind: &str,
    artifact: &str,
    checkpoint: Option<&str>,
    sample: Option<&str>,
) -> Value {
    json!({"artifact_kind": artifact, "kind": kind, "checkpoint_id": checkpoint,
           "sample_filename": sample, "prompt": format!("prompt {artifact}"),
           "scores": {"mert_style": null, "mert_train": null, "train_match": null,
                      "melody": 0.41, "melody_match": "late_bus_home.mp3", "quality": null}})
}

async fn run_detail(app: &TestApp, cookie: &str, run_id: &str) -> Reply {
    app.call(
        "GET",
        &format!("/api/v1/training-runs/{run_id}"),
        cookie,
        None,
    )
    .await
}

#[tokio::test]
async fn eval_complete_records_renders_and_scores() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    let s = lora_setup(&app, &admin, &carol).await;
    let created = start_snapshot_run(&app, &carol, &s).await;
    let e = claim_eval(&app, &s, &created, &[1, 2]).await;
    for i in 0..5 {
        let r = upload_eval(&app, &s, &e, &format!("eval_{i:02}"), "x.mp3").await;
        assert_eq!(
            r.status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&r.bytes)
        );
    }
    let renders = json!([
        eval_render("base", "eval_00", None, None),
        eval_render("prompt", "eval_01", Some(&e.cps[0]), None),
        eval_render("held_out", "eval_02", Some(&e.cps[0]), Some("held.mp3")),
        eval_render("prompt", "eval_03", Some(&e.cps[1]), None),
        eval_render("held_out", "eval_04", Some(&e.cps[1]), Some("held.mp3")),
    ]);
    let checkpoints = json!([
        {"id": e.cps[0], "style": 0.5, "train": 0.4, "train_match": "z.mp3",
         "melody": 0.2, "melody_match": "y.mp3", "quality": 5.0},
        {"id": e.cps[1], "style": 0.82, "train": 0.71, "train_match": "a.mp3",
         "melody": 0.44, "melody_match": "b.mp3", "quality": 6.9},
    ]);
    let r = complete_eval_job(
        &app,
        &s,
        &e,
        json!({"renders": renders, "checkpoints": checkpoints,
               "scorers": {"mert": true, "audiobox": true}}),
    )
    .await;
    assert_eq!(
        r.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&r.bytes)
    );

    let d = run_detail(&app, &carol, &e.run_id).await.json();
    let renders = d["renders"].as_array().unwrap();
    assert_eq!(renders.len(), 5);
    assert!(renders.iter().all(|r| r["artifact_id"].is_string()));
    assert_eq!(renders[0]["kind"], "base");
    assert!(renders[0]["checkpoint_id"].is_null());
    assert_eq!(renders[2]["sample_filename"], "held.mp3");
    assert_eq!(renders[2]["checkpoint_id"], e.cps[0].as_str());
    assert_eq!(renders[1]["prompt"], "prompt eval_01");
    assert_eq!(renders[1]["scores"]["melody"], 0.41);
    let art: String =
        sqlx::query_scalar("SELECT id FROM artifacts WHERE job_id = ? AND kind = 'eval_03'")
            .bind(&e.job_id)
            .fetch_one(&app.state.db)
            .await
            .unwrap();
    assert_eq!(renders[3]["artifact_id"], art.as_str());

    let cp2 = &d["checkpoints"][1];
    assert_eq!(cp2["epoch"], 2);
    assert_eq!(cp2["style_score"], 0.82);
    assert_eq!(cp2["train_score"], 0.71);
    assert_eq!(cp2["train_match"], "a.mp3");
    assert_eq!(cp2["melody_score"], 0.44);
    assert_eq!(cp2["melody_match"], "b.mp3");
    assert_eq!(cp2["quality_score"], 6.9);
    assert_eq!(d["checkpoints"][0]["style_score"], 0.5);
    assert_eq!(d["eval"]["status"], "completed");
    assert_eq!(d["eval"]["scorers"]["mert"], true);
    // The rendered mp3 is playable through the artifact endpoint.
    let r = app
        .call(
            "GET",
            &format!("/api/v1/artifacts/{art}/content"),
            &carol,
            None,
        )
        .await;
    assert_eq!(r.status, StatusCode::OK);
}

#[tokio::test]
async fn eval_complete_rejects_unknown_artifact_or_checkpoint() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    let s = lora_setup(&app, &admin, &carol).await;
    let created = start_snapshot_run(&app, &carol, &s).await;
    let e = claim_eval(&app, &s, &created, &[1]).await;
    assert_eq!(
        upload_eval(&app, &s, &e, "eval_00", "x.mp3").await.status,
        StatusCode::OK
    );
    let count = || async {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM lora_eval_renders")
            .fetch_one(&app.state.db)
            .await
            .unwrap()
    };

    // A valid first render followed by an artifact that was never uploaded: nothing is written.
    let r = complete_eval_job(
        &app,
        &s,
        &e,
        json!({"renders": [eval_render("base", "eval_00", None, None),
                           eval_render("prompt", "eval_09", Some(&e.cps[0]), None)],
               "checkpoints": [{"id": e.cps[0], "style": 0.9}]}),
    )
    .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(String::from_utf8_lossy(&r.bytes).contains("kết quả đánh giá không hợp lệ"));
    assert_eq!(count().await, 0);
    let style: Option<f64> =
        sqlx::query_scalar("SELECT style_score FROM lora_checkpoints WHERE id = ?")
            .bind(&e.cps[0])
            .fetch_one(&app.state.db)
            .await
            .unwrap();
    assert_eq!(style, None);

    // A checkpoint of another LoRA.
    let other_model: String = sqlx::query_scalar("SELECT base_model_id FROM loras WHERE id = ?")
        .bind(&e.lora_id)
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO loras (id, key, name, base_model_id, status, created_at) \
         VALUES ('other', 'other-key', 'Other', ?, 'evaluation', '2026-01-01T00:00:00Z')",
    )
    .bind(&other_model)
    .execute(&app.state.db)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO lora_checkpoints (id, lora_id, epoch, node_path, sha256, created_at) \
         VALUES ('foreign', 'other', 1, '/y', 'aa', 't')",
    )
    .execute(&app.state.db)
    .await
    .unwrap();
    let r = complete_eval_job(
        &app,
        &s,
        &e,
        json!({"renders": [eval_render("prompt", "eval_00", Some("foreign"), None)]}),
    )
    .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(count().await, 0);
    let r = complete_eval_job(
        &app,
        &s,
        &e,
        json!({"renders": [], "checkpoints": [{"id": "foreign", "style": 0.9}]}),
    )
    .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    let foreign: Option<f64> =
        sqlx::query_scalar("SELECT style_score FROM lora_checkpoints WHERE id = 'foreign'")
            .fetch_one(&app.state.db)
            .await
            .unwrap();
    assert_eq!(foreign, None);
    // The job is still running: the worker may retry with a corrected result.
    let status: String = sqlx::query_scalar("SELECT status FROM jobs WHERE id = ?")
        .bind(&e.job_id)
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    assert_eq!(status, "preparing");
}

#[tokio::test]
async fn eval_upload_kind_rules() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    let s = lora_setup(&app, &admin, &carol).await;
    let created = start_snapshot_run(&app, &carol, &s).await;
    let (_run, _) = finish_snapshot_run(&app, &s, &created, &[1]).await;
    // A generate job goes first (priority 0): eval kinds are refused for it.
    let owner: String = sqlx::query_scalar("SELECT id FROM users WHERE username = 'admin'")
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO jobs (id, type, owner_id, title, payload, created_at, updated_at) \
         VALUES ('gen1', 'generate', ?, 't', '{}', 't', 't')",
    )
    .bind(&owner)
    .execute(&app.state.db)
    .await
    .unwrap();
    let g = app.claim(&s.token).await.json();
    assert_eq!(g["id"], "gen1");
    let data = b"fake mp3";
    let r = app
        .upload(
            &s.token,
            "gen1",
            g["lease_token"].as_str().unwrap(),
            "eval_07",
            "x.mp3",
            data,
            &sha256_hex(data),
        )
        .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    register_eval_node(&app, &s).await;
    sqlx::query("UPDATE jobs SET status = 'completed' WHERE id = 'gen1'")
        .execute(&app.state.db)
        .await
        .unwrap();

    let job = app.claim(&s.token).await.json();
    assert_eq!(job["type"], "eval_lora");
    let e = EvalRun {
        run_id: String::new(),
        lora_id: String::new(),
        job_id: job["id"].as_str().unwrap().to_string(),
        lease: job["lease_token"].as_str().unwrap().to_string(),
        cps: vec![],
    };
    for (kind, file) in [
        ("eval_7", "x.mp3"),
        ("eval_007", "x.mp3"),
        ("eval_0a", "x.mp3"),
        ("eval_07", "x.wav"),
        ("eval_07", "x"),
    ] {
        let r = upload_eval(&app, &s, &e, kind, file).await;
        assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{kind} {file}");
    }
    let r = upload_eval(&app, &s, &e, "eval_07", "x.MP3").await;
    assert_eq!(r.status, StatusCode::OK);
    let (ct, key): (String, String) = sqlx::query_as(
        "SELECT content_type, storage_key FROM artifacts WHERE job_id = ? AND kind = 'eval_07'",
    )
    .bind(&e.job_id)
    .fetch_one(&app.state.db)
    .await
    .unwrap();
    assert_eq!(ct, "audio/mpeg");
    assert!(key.ends_with("/eval_07.mp3"), "{key}");
}

#[tokio::test]
async fn run_detail_exposes_review_fields_without_paths() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    let s = lora_setup(&app, &admin, &carol).await;
    let created = start_snapshot_run(&app, &carol, &s).await;
    let e = claim_eval(&app, &s, &created, &[1, 2]).await;

    // While the eval job is running the detail shows it.
    let r = run_detail(&app, &carol, &e.run_id).await;
    assert_eq!(r.status, StatusCode::OK);
    let body = String::from_utf8_lossy(&r.bytes).to_string();
    assert!(
        !body.contains("node_path") && !body.contains("/x/epoch_"),
        "{body}"
    );
    let d = r.json();
    assert_eq!(d["lora"]["id"], e.lora_id.as_str());
    assert_eq!(d["lora"]["key"], created["lora_key"]);
    assert_eq!(d["lora"]["base_key"], created["lora_key"]);
    assert!(d["lora"]["chosen_checkpoint_id"].is_null());
    assert_eq!(d["eval"]["job_id"], e.job_id.as_str());
    assert_eq!(d["eval"]["status"], "preparing");
    assert_eq!(d["eval"]["scorers"]["mert"], false);
    assert_eq!(d["renders"], json!([]));
    assert!(d["style_preset"].is_null());
    assert!(d["files_job"].is_null());
    let held = d["held_out_tracks"].as_array().unwrap();
    assert_eq!(held.len(), 1);
    assert!(held[0]["filename"].is_string());
    let snap_art: String = sqlx::query_scalar(
        "SELECT json_extract(content, '$.held_out[0].artifact_id') FROM dataset_snapshots",
    )
    .fetch_one(&app.state.db)
    .await
    .unwrap();
    assert_eq!(held[0]["artifact_id"], snap_art.as_str());
    let c0 = &d["checkpoints"][0];
    assert_eq!(c0["note"], "");
    assert!(c0["expires_at"].is_null());
    assert!(c0["delete_requested_at"].is_null());
    assert!(c0["style_score"].is_null());

    // A trained Style on this LoRA and a file job show up too.
    sqlx::query(
        "INSERT INTO style_presets (id, genre_id, name, kind, lora_id, lora_scale, is_active) \
         SELECT 'sp1', id, 'My Style', 'trained', ?, 0.7, 0 FROM genres LIMIT 1",
    )
    .bind(&e.lora_id)
    .execute(&app.state.db)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO jobs (id, type, owner_id, title, payload, status, error, created_at, updated_at) \
         VALUES ('f1', 'lora_files', (SELECT id FROM users LIMIT 1), 't', ?, 'failed', 'boom', 't1', 't1')",
    )
    .bind(json!({"lora": {"id": e.lora_id}, "op": "publish", "node_path": "/secret/x"}).to_string())
    .execute(&app.state.db)
    .await
    .unwrap();
    let r = run_detail(&app, &carol, &e.run_id).await;
    let body = String::from_utf8_lossy(&r.bytes).to_string();
    assert!(!body.contains("/secret/x"), "{body}");
    let d = r.json();
    assert_eq!(d["style_preset"]["id"], "sp1");
    assert_eq!(d["style_preset"]["name"], "My Style");
    assert_eq!(d["style_preset"]["lora_scale"], 0.7);
    assert_eq!(d["style_preset"]["is_active"], false);
    assert_eq!(d["files_job"]["job_id"], "f1");
    assert_eq!(d["files_job"]["status"], "failed");
    assert_eq!(d["files_job"]["op"], "publish");
    assert_eq!(d["files_job"]["error"], "boom");
}

#[tokio::test]
async fn eval_with_missing_scorers_keeps_nulls() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    let s = lora_setup(&app, &admin, &carol).await;
    let created = start_snapshot_run(&app, &carol, &s).await;
    let e = claim_eval(&app, &s, &created, &[1]).await;
    assert_eq!(
        upload_eval(&app, &s, &e, "eval_00", "x.mp3").await.status,
        StatusCode::OK
    );
    let r = complete_eval_job(
        &app,
        &s,
        &e,
        json!({"renders": [eval_render("base", "eval_00", None, None)],
               "checkpoints": [{"id": e.cps[0], "style": null, "train": null, "train_match": null,
                                "melody": 0.44, "melody_match": "b.mp3", "quality": null}],
               "scorers": {"mert": false, "audiobox": false}}),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    let d = run_detail(&app, &carol, &e.run_id).await.json();
    let c = &d["checkpoints"][0];
    assert!(c["quality_score"].is_null());
    assert!(c["style_score"].is_null());
    assert!(c["train_match"].is_null());
    assert_eq!(c["melody_score"], 0.44);
    assert_eq!(d["eval"]["scorers"]["audiobox"], false);
    assert_eq!(d["eval"]["scorers"]["mert"], false);
    assert!(d["renders"][0]["scores"]["quality"].is_null());
}

// ---------- LoRA review: note, publish, cleanup, expiry, delete ----------

struct Reviewed {
    s: LoraSetup,
    carol: String,
    admin: String,
    lora_id: String,
    lora_key: String,
    node_id: String,
    /// Checkpoint ids: epoch 1, epoch 2.
    cps: Vec<String>,
}

/// A finished snapshot run with epochs 1 and 2, on a node that also takes `lora_files`.
async fn reviewed_lora(app: &TestApp) -> Reviewed {
    let admin = app.login("admin").await;
    let carol = app.login("carol").await;
    let s = lora_setup(app, &admin, &carol).await;
    let created = start_snapshot_run(app, &carol, &s).await;
    let (run_id, node_id) = finish_snapshot_run(app, &s, &created, &[1, 2]).await;
    let reg = json!({
        "protocol_version": 1, "worker_version": "test", "fresh_start": true,
        "job_types": ["generate", "train_lora", "eval_lora", "lora_files"],
        "engines": [{"name": "acestep", "version": "1", "capabilities": ["generate", "lora"]}],
        "models": [{"engine": "acestep", "key": "acestep-v15-xl-sft", "capabilities": ["generate", "lora"]}],
        "loras": [], "datasets": [], "gpu": null,
    });
    let r = app
        .call("POST", "/api/worker/v1/register", &s.token, Some(reg))
        .await;
    assert_eq!(r.status, StatusCode::OK);
    let lora_id: String = sqlx::query_scalar("SELECT id FROM loras WHERE job_id = ?")
        .bind(&run_id)
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    let cps: Vec<String> =
        sqlx::query_scalar("SELECT id FROM lora_checkpoints WHERE lora_id = ? ORDER BY epoch")
            .bind(&lora_id)
            .fetch_all(&app.state.db)
            .await
            .unwrap();
    Reviewed {
        lora_key: created["lora_key"].as_str().unwrap().to_string(),
        s,
        carol,
        admin,
        lora_id,
        node_id,
        cps,
    }
}

async fn publish(app: &TestApp, r: &Reviewed, cp: &str, name: &str, scale: f64) -> Reply {
    app.call(
        "POST",
        &format!("/api/v1/loras/{}/publish", r.lora_id),
        &r.carol,
        Some(json!({"checkpoint_id": cp, "style_name": name, "lora_scale": scale})),
    )
    .await
}

/// (id, status, priority, payload) of every lora_files job, oldest first.
async fn files_jobs(app: &TestApp) -> Vec<(String, String, i64, Value)> {
    let rows: Vec<(String, String, i64, String)> = sqlx::query_as(
        "SELECT id, status, priority, payload FROM jobs WHERE type = 'lora_files' ORDER BY created_at, id",
    )
    .fetch_all(&app.state.db)
    .await
    .unwrap();
    rows.into_iter()
        .map(|(i, s, p, j)| (i, s, p, serde_json::from_str(&j).unwrap()))
        .collect()
}

/// Let the node claim the next job (must be lora_files). Returns (job id, lease token).
async fn claim_files(app: &TestApp, r: &Reviewed) -> (String, String) {
    let job = app.claim(&r.s.token).await;
    assert_eq!(job.status, StatusCode::OK);
    let job = job.json();
    assert_eq!(job["type"], "lora_files");
    (
        job["id"].as_str().unwrap().to_string(),
        job["lease_token"].as_str().unwrap().to_string(),
    )
}

async fn complete_files_job(
    app: &TestApp,
    r: &Reviewed,
    job: &(String, String),
    result: Value,
) -> Reply {
    app.call(
        "POST",
        &format!("/api/worker/v1/jobs/{}/complete", job.0),
        &r.s.token,
        Some(json!({"lease_token": job.1, "result": result})),
    )
    .await
}

/// Claim the next lora_files job and complete it with `result`.
async fn run_files_job(app: &TestApp, r: &Reviewed, result: Value) -> Reply {
    let job = claim_files(app, r).await;
    complete_files_job(app, r, &job, result).await
}

async fn heartbeat_of(app: &TestApp, token: &str) -> Reply {
    app.call("POST", "/api/worker/v1/heartbeat", token, Some(json!({})))
        .await
}

async fn cp_row(app: &TestApp, id: &str) -> (String, Option<String>, Option<String>) {
    sqlx::query_as(
        "SELECT status, expires_at, delete_requested_at FROM lora_checkpoints WHERE id = ?",
    )
    .bind(id)
    .fetch_one(&app.state.db)
    .await
    .unwrap()
}

async fn styles_of(app: &TestApp, lora_id: &str) -> Vec<(String, f64, bool, String)> {
    sqlx::query_as(
        "SELECT name, lora_scale, is_active, genre_id FROM style_presets WHERE lora_id = ? AND kind = 'trained'",
    )
    .bind(lora_id)
    .fetch_all(&app.state.db)
    .await
    .unwrap()
}

fn published_result(key: &str) -> Value {
    json!({"published": {"key": key, "sha256": "a".repeat(64)}})
}

#[tokio::test]
async fn note_is_saved_and_limited() {
    let app = TestApp::new().await;
    let r = reviewed_lora(&app).await;
    let url = format!("/api/v1/lora-checkpoints/{}", r.cps[1]);
    let ok = app
        .call(
            "PATCH",
            &url,
            &r.carol,
            Some(json!({"note": "nghe ấm, hơi tối"})),
        )
        .await;
    assert_eq!(ok.status, StatusCode::OK);
    assert_eq!(ok.json()["note"], "nghe ấm, hơi tối");
    assert_eq!(ok.json()["epoch"], 2);
    assert!(ok.json().get("node_path").is_none());
    let note: String = sqlx::query_scalar("SELECT note FROM lora_checkpoints WHERE id = ?")
        .bind(&r.cps[1])
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    assert_eq!(note, "nghe ấm, hơi tối");
    // 500 characters (not bytes) are fine, 501 are not.
    let max = "ố".repeat(500);
    let r500 = app
        .call("PATCH", &url, &r.carol, Some(json!({"note": max})))
        .await;
    assert_eq!(r500.status, StatusCode::OK);
    let r501 = app
        .call(
            "PATCH",
            &url,
            &r.carol,
            Some(json!({"note": "x".repeat(501)})),
        )
        .await;
    assert_eq!(r501.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        r501.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Ghi chú tối đa 500 ký tự")
    );
    // A plain user cannot.
    let dave = app.login("bob").await;
    let denied = app
        .call("PATCH", &url, &dave, Some(json!({"note": "x"})))
        .await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
}

async fn finish_eval(app: &TestApp) {
    sqlx::query("UPDATE jobs SET status = 'completed' WHERE type = 'eval_lora'")
        .execute(&app.state.db)
        .await
        .unwrap();
}

/// (key, base_key, chosen_checkpoint_id, status) of the LoRA.
async fn lora_row(app: &TestApp, id: &str) -> (String, Option<String>, Option<String>, String) {
    sqlx::query_as("SELECT key, base_key, chosen_checkpoint_id, status FROM loras WHERE id = ?")
        .bind(id)
        .fetch_one(&app.state.db)
        .await
        .unwrap()
}

async fn installs(app: &TestApp, lora_id: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM lora_installations WHERE lora_id = ?")
        .bind(lora_id)
        .fetch_one(&app.state.db)
        .await
        .unwrap()
}

/// Publish `epoch` (1 or 2) and let the node finish the copy.
async fn publish_and_complete(app: &TestApp, r: &Reviewed, epoch: usize, name: &str) {
    let res = publish(app, r, &r.cps[epoch - 1], name, 0.7).await;
    assert_eq!(res.status, StatusCode::ACCEPTED);
    let key = format!("{}-e{epoch}", r.lora_key);
    let done = run_files_job(app, r, published_result(&key)).await;
    assert_eq!(done.status, StatusCode::OK);
}

#[tokio::test]
async fn publish_queues_job_and_style_but_changes_nothing_until_complete() {
    let app = TestApp::new().await;
    let r = reviewed_lora(&app).await;
    let res = publish(&app, &r, &r.cps[1], "Lo-fi của tôi", 0.7).await;
    assert_eq!(
        res.status,
        StatusCode::ACCEPTED,
        "{}",
        String::from_utf8_lossy(&res.bytes)
    );
    let body = res.json();
    // Nothing about the LoRA or its checkpoints moved yet.
    for cp in &r.cps {
        assert_eq!(cp_row(&app, cp).await, ("kept".to_string(), None, None));
    }
    let (key, base, chosen, status) = lora_row(&app, &r.lora_id).await;
    assert_eq!(key, r.lora_key);
    assert_eq!((base, chosen), (None, None));
    assert_eq!(status, "evaluation");

    let styles = styles_of(&app, &r.lora_id).await;
    assert_eq!(styles.len(), 1);
    assert_eq!(styles[0].0, "Lo-fi của tôi");
    assert_eq!(styles[0].1, 0.7);
    assert!(!styles[0].2);
    let sid: String = sqlx::query_scalar("SELECT id FROM style_presets WHERE lora_id = ?")
        .bind(&r.lora_id)
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    assert_eq!(body["style_preset_id"], sid);
    let (model, tmpl): (Option<String>, String) =
        sqlx::query_as("SELECT model_id, prompt_template FROM style_presets WHERE id = ?")
            .bind(&sid)
            .fetch_one(&app.state.db)
            .await
            .unwrap();
    assert_eq!(model.as_deref(), Some(r.s.model_id.as_str()));
    // Keyword first, as in training ("lushlofi, <caption>").
    assert_eq!(tmpl, "lushlofi, {prompt}");
    let compose =
        |prompt: &str| lush_api::routes::jobs::compose_prompt(prompt, &tmpl, &[], &[], false);
    assert_eq!(compose("rainy night"), "lushlofi, rainy night");
    assert_eq!(compose("  "), "lushlofi");

    let jobs = files_jobs(&app).await;
    assert_eq!(jobs.len(), 1);
    assert_eq!(body["job_id"], jobs[0].0);
    assert_eq!(jobs[0].2, 10);
    let p = &jobs[0].3;
    assert_eq!(p["op"], "publish");
    assert_eq!(p["lora_id"], r.lora_id);
    assert_eq!(p["lora"]["id"], r.lora_id);
    assert_eq!(p["node_id"], r.node_id);
    assert_eq!(p["target_key"], format!("{}-e2", r.lora_key));
    assert_eq!(p["checkpoint"]["id"], r.cps[1]);
    assert_eq!(p["checkpoint"]["path"], "/x/epoch_2");
    assert_eq!(p["checkpoint"]["sha256"], "a".repeat(64));

    // The node reports the copy: now the choice takes effect.
    let key2 = format!("{}-e2", r.lora_key);
    let done = run_files_job(&app, &r, published_result(&key2)).await;
    assert_eq!(done.status, StatusCode::OK);
    let (c2, c2_exp, _) = cp_row(&app, &r.cps[1]).await;
    assert_eq!((c2.as_str(), c2_exp), ("chosen", None));
    let (c1, c1_exp, _) = cp_row(&app, &r.cps[0]).await;
    assert_eq!(c1, "kept");
    let exp = chrono::DateTime::parse_from_rfc3339(&c1_exp.expect("kept gets a deadline")).unwrap();
    let hours = (exp.with_timezone(&chrono::Utc) - chrono::Utc::now()).num_hours();
    assert!((29 * 24..=30 * 24).contains(&hours), "{hours}");
    let (key, base, chosen, status) = lora_row(&app, &r.lora_id).await;
    assert_eq!(key, key2);
    assert_eq!(base.as_deref(), Some(r.lora_key.as_str()));
    assert_eq!(chosen.as_deref(), Some(r.cps[1].as_str()));
    assert_eq!(status, "approved");
    assert_eq!(installs(&app, &r.lora_id).await, 1);
}

#[tokio::test]
async fn failed_publish_job_leaves_everything_untouched_and_can_be_retried() {
    let app = TestApp::new().await;
    let r = reviewed_lora(&app).await;
    publish_and_complete(&app, &r, 2, "Lo-fi").await;
    let key2 = format!("{}-e2", r.lora_key);
    // Choose epoch 1: the job fails on the node.
    assert_eq!(
        publish(&app, &r, &r.cps[0], "Lo-fi", 0.7).await.status,
        StatusCode::ACCEPTED
    );
    let job = claim_files(&app, &r).await;
    let failed = app
        .call(
            "POST",
            &format!("/api/worker/v1/jobs/{}/fail", job.0),
            &r.s.token,
            Some(json!({"lease_token": job.1, "error": "copy failed", "retryable": false})),
        )
        .await;
    assert_eq!(failed.status, StatusCode::OK);
    // The old adapter still serves.
    let (key, _, chosen, status) = lora_row(&app, &r.lora_id).await;
    assert_eq!(key, key2);
    assert_eq!(chosen.as_deref(), Some(r.cps[1].as_str()));
    assert_eq!(status, "approved");
    assert_eq!(installs(&app, &r.lora_id).await, 1);
    assert_eq!(cp_row(&app, &r.cps[1]).await.0, "chosen");
    assert_eq!(cp_row(&app, &r.cps[0]).await.0, "kept");
    // Posting again is the retry.
    let again = publish(&app, &r, &r.cps[0], "Lo-fi", 0.7).await;
    assert_eq!(again.status, StatusCode::ACCEPTED);
    assert_eq!(styles_of(&app, &r.lora_id).await.len(), 1);
}

#[tokio::test]
async fn publishing_the_installed_choice_only_updates_the_style() {
    let app = TestApp::new().await;
    let r = reviewed_lora(&app).await;
    publish_and_complete(&app, &r, 2, "Lo-fi").await;
    let jobs_before = files_jobs(&app).await.len();
    let res = publish(&app, &r, &r.cps[1], "Tên mới", 0.5).await;
    assert_eq!(res.status, StatusCode::OK);
    assert!(res.json()["job_id"].is_null());
    let styles = styles_of(&app, &r.lora_id).await;
    assert_eq!(styles.len(), 1);
    assert_eq!(
        (styles[0].0.as_str(), styles[0].1, styles[0].2),
        ("Tên mới", 0.5, true)
    );
    assert_eq!(files_jobs(&app).await.len(), jobs_before);
}

#[tokio::test]
async fn publish_twice_while_files_job_live_is_409() {
    let app = TestApp::new().await;
    let r = reviewed_lora(&app).await;
    assert_eq!(
        publish(&app, &r, &r.cps[1], "A", 0.7).await.status,
        StatusCode::ACCEPTED
    );
    let second = publish(&app, &r, &r.cps[0], "B", 0.5).await;
    assert_eq!(second.status, StatusCode::CONFLICT);
    assert!(
        second.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("LoRA đang được cập nhật trên máy, chờ một chút")
    );
    assert_eq!(styles_of(&app, &r.lora_id).await.len(), 1);
    assert_eq!(styles_of(&app, &r.lora_id).await[0].0, "A");
    assert_eq!(files_jobs(&app).await.len(), 1);
    // The database refuses a second live file job of the LoRA even if the check is raced.
    let dup = sqlx::query(
        "INSERT INTO jobs (id, type, priority, owner_id, title, payload, created_at, updated_at) \
         SELECT 'dup', 'lora_files', 10, owner_id, 't', payload, 't', 't' FROM jobs WHERE type = 'lora_files'",
    )
    .execute(&app.state.db)
    .await;
    assert!(dup.is_err());
}

#[tokio::test]
async fn files_publish_complete_approves_and_activates() {
    let app = TestApp::new().await;
    let r = reviewed_lora(&app).await;
    publish(&app, &r, &r.cps[1], "Lo-fi", 0.7).await;
    let key = format!("{}-e2", r.lora_key);
    let job = claim_files(&app, &r).await;
    // A wrong sha is rejected and changes nothing.
    let bad = complete_files_job(
        &app,
        &r,
        &job,
        json!({"published": {"key": key, "sha256": "b".repeat(64)}}),
    )
    .await;
    assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY);
    let (k, _, chosen, status) = lora_row(&app, &r.lora_id).await;
    assert_eq!(
        (k, chosen, status.as_str()),
        (r.lora_key.clone(), None, "evaluation")
    );
    assert!(!styles_of(&app, &r.lora_id).await[0].2);
    assert_eq!(cp_row(&app, &r.cps[1]).await.0, "kept");
    // A wrong key is rejected too.
    let bad_key = complete_files_job(&app, &r, &job, published_result("other-key")).await;
    assert_eq!(bad_key.status, StatusCode::UNPROCESSABLE_ENTITY);
    // The right answer approves, activates and records the installation.
    let ok = complete_files_job(&app, &r, &job, published_result(&key)).await;
    assert_eq!(
        ok.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&ok.bytes)
    );
    assert_eq!(lora_row(&app, &r.lora_id).await.3, "approved");
    assert!(styles_of(&app, &r.lora_id).await[0].2);
    let inst: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM lora_installations WHERE lora_id = ? AND node_id = ?",
    )
    .bind(&r.lora_id)
    .bind(&r.node_id)
    .fetch_one(&app.state.db)
    .await
    .unwrap();
    assert_eq!(inst, 1);
}

#[tokio::test]
async fn changing_epoch_updates_same_style() {
    let app = TestApp::new().await;
    let r = reviewed_lora(&app).await;
    publish_and_complete(&app, &r, 2, "Lo-fi").await;
    let again = publish(&app, &r, &r.cps[0], "Lo-fi đổi tên", 0.55).await;
    assert_eq!(again.status, StatusCode::ACCEPTED);
    // Until the copy is reported, epoch 2 keeps serving.
    let k2 = format!("{}-e2", r.lora_key);
    assert_eq!(lora_row(&app, &r.lora_id).await.0, k2);
    assert_eq!(cp_row(&app, &r.cps[1]).await.0, "chosen");
    assert_eq!(installs(&app, &r.lora_id).await, 1);
    let styles = styles_of(&app, &r.lora_id).await;
    assert_eq!(styles.len(), 1);
    assert_eq!((styles[0].0.as_str(), styles[0].1), ("Lo-fi đổi tên", 0.55));
    assert!(styles[0].2, "an existing Style keeps its is_active");

    let k1 = format!("{}-e1", r.lora_key);
    assert_eq!(
        run_files_job(&app, &r, published_result(&k1)).await.status,
        StatusCode::OK
    );
    let (key, base, chosen, _) = lora_row(&app, &r.lora_id).await;
    assert_eq!(key, k1);
    assert_eq!(base.as_deref(), Some(r.lora_key.as_str()));
    assert_eq!(chosen.as_deref(), Some(r.cps[0].as_str()));
    let (s2, exp2, _) = cp_row(&app, &r.cps[1]).await;
    assert_eq!(s2, "kept");
    assert!(exp2.is_some());
    assert_eq!(cp_row(&app, &r.cps[0]).await.0, "chosen");
    assert_eq!(installs(&app, &r.lora_id).await, 1);
    assert_eq!(styles_of(&app, &r.lora_id).await.len(), 1);
}

#[tokio::test]
async fn cleanup_requires_a_chosen_checkpoint() {
    let app = TestApp::new().await;
    let r = reviewed_lora(&app).await;
    let res = app
        .call(
            "POST",
            &format!("/api/v1/loras/{}/cleanup", r.lora_id),
            &r.carol,
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert!(
        res.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Chọn một bản trước khi xóa các bản còn lại")
    );
    let marked: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM lora_checkpoints WHERE delete_requested_at IS NOT NULL",
    )
    .fetch_one(&app.state.db)
    .await
    .unwrap();
    assert_eq!(marked, 0);
    assert!(files_jobs(&app).await.is_empty());
}

async fn cleanup_of(app: &TestApp, r: &Reviewed) -> Reply {
    app.call(
        "POST",
        &format!("/api/v1/loras/{}/cleanup", r.lora_id),
        &r.carol,
        None,
    )
    .await
}

#[tokio::test]
async fn cleanup_marks_kept_and_queues_delete() {
    let app = TestApp::new().await;
    let r = reviewed_lora(&app).await;
    publish_and_complete(&app, &r, 2, "Lo-fi").await;
    // The listening job of the run is still waiting: not yet.
    let wait = cleanup_of(&app, &r).await;
    assert_eq!(wait.status, StatusCode::CONFLICT);
    assert!(
        wait.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Đang tạo nhạc nghe thử, chờ xong rồi xóa")
    );
    assert!(cp_row(&app, &r.cps[0]).await.2.is_none());
    finish_eval(&app).await;
    let res = cleanup_of(&app, &r).await;
    assert_eq!(res.status, StatusCode::ACCEPTED);
    assert_eq!(res.json()["count"], 1);
    assert!(cp_row(&app, &r.cps[0]).await.2.is_some());
    assert!(cp_row(&app, &r.cps[1]).await.2.is_none());
    let jobs = files_jobs(&app).await;
    let del = jobs.iter().find(|j| j.3["op"] == "delete").unwrap();
    assert_eq!(res.json()["job_id"], del.0);
    assert_eq!(del.3["lora_id"], r.lora_id);
    assert_eq!(del.3["node_id"], r.node_id);
    assert_eq!(
        del.3["checkpoints"],
        json!([{"id": r.cps[0], "path": "/x/epoch_1"}])
    );
    // A second click while that job waits is refused.
    let twice = cleanup_of(&app, &r).await;
    assert_eq!(twice.status, StatusCode::CONFLICT);
    // The node reports one deleted, the chosen is untouched.
    let done = run_files_job(&app, &r, json!({"deleted": [r.cps[0]], "missing": []})).await;
    assert_eq!(
        done.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&done.bytes)
    );
    assert_eq!(cp_row(&app, &r.cps[0]).await.0, "deleted");
    assert_eq!(cp_row(&app, &r.cps[1]).await.0, "chosen");
    // Nothing left to delete.
    let none = cleanup_of(&app, &r).await;
    assert_eq!(none.status, StatusCode::OK);
    assert_eq!(none.json()["count"], 0);
    assert!(none.json()["job_id"].is_null());
}

/// Claim the next lora_files job and fail it (the node could not delete).
async fn fail_files_job(app: &TestApp, r: &Reviewed) {
    let job = claim_files(app, r).await;
    let failed = app
        .call(
            "POST",
            &format!("/api/worker/v1/jobs/{}/fail", job.0),
            &r.s.token,
            Some(json!({"lease_token": job.1, "error": "disk busy", "retryable": false})),
        )
        .await;
    assert_eq!(failed.status, StatusCode::OK);
}

#[tokio::test]
async fn failed_delete_job_is_queued_again_by_cleanup() {
    let app = TestApp::new().await;
    let r = reviewed_lora(&app).await;
    publish_and_complete(&app, &r, 2, "Lo-fi").await;
    finish_eval(&app).await;
    assert_eq!(cleanup_of(&app, &r).await.status, StatusCode::ACCEPTED);
    fail_files_job(&app, &r).await;
    // Marked, not deleted, no live job: abandoned. Cleanup picks it up again.
    assert!(cp_row(&app, &r.cps[0]).await.2.is_some());
    let again = cleanup_of(&app, &r).await;
    assert_eq!(again.status, StatusCode::ACCEPTED);
    assert_eq!(again.json()["count"], 1);
    let jobs = files_jobs(&app).await;
    let live = jobs.iter().find(|j| j.0 == again.json()["job_id"]).unwrap();
    assert_eq!(live.3["checkpoints"][0]["id"], r.cps[0]);
}

#[tokio::test]
async fn heartbeat_requeues_old_abandoned_marks_even_when_archived() {
    let app = TestApp::new().await;
    let r = reviewed_lora(&app).await;
    publish_and_complete(&app, &r, 2, "Lo-fi").await;
    finish_eval(&app).await;
    assert_eq!(cleanup_of(&app, &r).await.status, StatusCode::ACCEPTED);
    fail_files_job(&app, &r).await;
    sqlx::query("UPDATE loras SET status = 'archived' WHERE id = ?")
        .bind(&r.lora_id)
        .execute(&app.state.db)
        .await
        .unwrap();
    // A fresh mark is left alone.
    assert_eq!(heartbeat_of(&app, &r.s.token).await.status, StatusCode::OK);
    assert_eq!(files_jobs(&app).await.len(), 2);
    sqlx::query(
        "UPDATE lora_checkpoints SET delete_requested_at = '2020-01-01T00:00:00.000Z' WHERE id = ?",
    )
    .bind(&r.cps[0])
    .execute(&app.state.db)
    .await
    .unwrap();
    assert_eq!(heartbeat_of(&app, &r.s.token).await.status, StatusCode::OK);
    let jobs = files_jobs(&app).await;
    assert_eq!(jobs.len(), 3);
    let last = jobs.last().unwrap();
    assert_eq!(last.1, "queued");
    assert_eq!(last.3["op"], "delete");
    assert_eq!(last.3["checkpoints"][0]["id"], r.cps[0]);
}

#[tokio::test]
async fn heartbeat_queues_delete_for_expired_checkpoints() {
    let app = TestApp::new().await;
    let r = reviewed_lora(&app).await;
    publish(&app, &r, &r.cps[1], "Lo-fi", 0.7).await;
    // Nothing is chosen yet and a file job is waiting: no expiry cleanup.
    sqlx::query("UPDATE lora_checkpoints SET expires_at = '2020-01-01T00:00:00.000Z' WHERE id = ?")
        .bind(&r.cps[0])
        .execute(&app.state.db)
        .await
        .unwrap();
    assert_eq!(heartbeat_of(&app, &r.s.token).await.status, StatusCode::OK);
    assert_eq!(files_jobs(&app).await.len(), 1, "publish job still live");
    let k2 = format!("{}-e2", r.lora_key);
    assert_eq!(
        run_files_job(&app, &r, published_result(&k2)).await.status,
        StatusCode::OK
    );
    assert_eq!(heartbeat_of(&app, &r.s.token).await.status, StatusCode::OK);
    let jobs = files_jobs(&app).await;
    let deletes: Vec<_> = jobs.iter().filter(|j| j.3["op"] == "delete").collect();
    assert_eq!(deletes.len(), 1);
    assert_eq!(deletes[0].3["node_id"], r.node_id);
    assert_eq!(deletes[0].3["checkpoints"][0]["id"], r.cps[0]);
    assert!(cp_row(&app, &r.cps[0]).await.2.is_some());
    // Heartbeat again while it waits: nothing new.
    assert_eq!(heartbeat_of(&app, &r.s.token).await.status, StatusCode::OK);
    assert_eq!(files_jobs(&app).await.len(), 2);
    // Finish it; the chosen epoch is never swept, even with a stale deadline.
    let done = run_files_job(&app, &r, json!({"deleted": [r.cps[0]], "missing": []})).await;
    assert_eq!(done.status, StatusCode::OK);
    sqlx::query("UPDATE lora_checkpoints SET expires_at = '2020-01-01T00:00:00.000Z' WHERE id = ?")
        .bind(&r.cps[1])
        .execute(&app.state.db)
        .await
        .unwrap();
    assert_eq!(heartbeat_of(&app, &r.s.token).await.status, StatusCode::OK);
    assert_eq!(cp_row(&app, &r.cps[1]).await.2, None);
    assert_eq!(cp_row(&app, &r.cps[1]).await.0, "chosen");
    assert_eq!(files_jobs(&app).await.len(), 2, "no new delete job");
}

#[tokio::test]
async fn heartbeat_of_another_node_queues_nothing() {
    let app = TestApp::new().await;
    let r = reviewed_lora(&app).await;
    publish_and_complete(&app, &r, 2, "Lo-fi").await;
    sqlx::query("UPDATE lora_checkpoints SET expires_at = '2020-01-01T00:00:00.000Z' WHERE id = ?")
        .bind(&r.cps[0])
        .execute(&app.state.db)
        .await
        .unwrap();
    let other = app
        .call(
            "POST",
            "/api/v1/nodes",
            &r.admin,
            Some(json!({"name": "other-node"})),
        )
        .await;
    let token = other.json()["token"].as_str().unwrap().to_string();
    let reg = json!({
        "protocol_version": 1, "worker_version": "test", "fresh_start": true,
        "job_types": ["lora_files"], "engines": [], "models": [], "loras": [],
        "datasets": [], "gpu": null,
    });
    let r2 = app
        .call("POST", "/api/worker/v1/register", &token, Some(reg))
        .await;
    assert_eq!(r2.status, StatusCode::OK);
    assert_eq!(heartbeat_of(&app, &token).await.status, StatusCode::OK);
    assert_eq!(files_jobs(&app).await.len(), 1, "only the publish job");
    // The owning node does queue it.
    assert_eq!(heartbeat_of(&app, &r.s.token).await.status, StatusCode::OK);
    assert_eq!(files_jobs(&app).await.len(), 2);
}

#[tokio::test]
async fn members_cannot_publish_cleanup_or_delete() {
    let app = TestApp::new().await;
    let r = reviewed_lora(&app).await;
    let bob = app.login("bob").await;
    let body = json!({"checkpoint_id": r.cps[1], "style_name": "x", "lora_scale": 0.7});
    let p = app
        .call(
            "POST",
            &format!("/api/v1/loras/{}/publish", r.lora_id),
            &bob,
            Some(body),
        )
        .await;
    assert_eq!(p.status, StatusCode::FORBIDDEN);
    let c = app
        .call(
            "POST",
            &format!("/api/v1/loras/{}/cleanup", r.lora_id),
            &bob,
            None,
        )
        .await;
    assert_eq!(c.status, StatusCode::FORBIDDEN);
    let d = app
        .call(
            "DELETE",
            &format!("/api/v1/loras/{}", r.lora_id),
            &bob,
            None,
        )
        .await;
    assert_eq!(d.status, StatusCode::FORBIDDEN);
    assert!(files_jobs(&app).await.is_empty());
    assert_eq!(lora_row(&app, &r.lora_id).await.3, "evaluation");
}

#[tokio::test]
async fn publish_validates_in_order() {
    let app = TestApp::new().await;
    let r = reviewed_lora(&app).await;
    let bad = |v: Value| {
        let url = format!("/api/v1/loras/{}/publish", r.lora_id);
        let (app, c) = (&app, r.carol.clone());
        async move { app.call("POST", &url, &c, Some(v)).await }
    };
    let name = bad(json!({"checkpoint_id": r.cps[1], "style_name": "  ", "lora_scale": 0.7})).await;
    assert_eq!(name.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        name.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Tên Style phải có 1–80 ký tự")
    );
    let long =
        bad(json!({"checkpoint_id": r.cps[1], "style_name": "x".repeat(81), "lora_scale": 0.7}))
            .await;
    assert_eq!(long.status, StatusCode::UNPROCESSABLE_ENTITY);
    let scale =
        bad(json!({"checkpoint_id": r.cps[1], "style_name": "ok", "lora_scale": 1.5})).await;
    assert_eq!(scale.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        scale.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Độ mạnh phải trong khoảng 0–1")
    );
    let nan = bad(json!({"checkpoint_id": r.cps[1], "style_name": "ok", "lora_scale": -0.1})).await;
    assert_eq!(nan.status, StatusCode::UNPROCESSABLE_ENTITY);
    // Foreign / deleted / delete-requested checkpoint.
    let foreign =
        bad(json!({"checkpoint_id": "nope", "style_name": "ok", "lora_scale": 0.7})).await;
    assert_eq!(foreign.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        foreign.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Bản lưu này không còn trên máy")
    );
    sqlx::query("UPDATE lora_checkpoints SET delete_requested_at = 'x' WHERE id = ?")
        .bind(&r.cps[0])
        .execute(&app.state.db)
        .await
        .unwrap();
    let gone = bad(json!({"checkpoint_id": r.cps[0], "style_name": "ok", "lora_scale": 0.7})).await;
    assert_eq!(gone.status, StatusCode::UNPROCESSABLE_ENTITY);
    // Unknown LoRA / archived LoRA.
    let missing = app
        .call(
            "POST",
            "/api/v1/loras/none/publish",
            &r.carol,
            Some(json!({"checkpoint_id": r.cps[1], "style_name": "ok", "lora_scale": 0.7})),
        )
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    sqlx::query("UPDATE loras SET status = 'archived' WHERE id = ?")
        .bind(&r.lora_id)
        .execute(&app.state.db)
        .await
        .unwrap();
    let archived =
        bad(json!({"checkpoint_id": r.cps[1], "style_name": "ok", "lora_scale": 0.7})).await;
    assert_eq!(archived.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        archived.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("LoRA đã bị xóa")
    );
    assert!(styles_of(&app, &r.lora_id).await.is_empty());
    assert!(files_jobs(&app).await.is_empty());
}

#[tokio::test]
async fn delete_lora_blocked_by_style_then_archives() {
    let app = TestApp::new().await;
    let r = reviewed_lora(&app).await;
    publish(&app, &r, &r.cps[1], "Lo-fi của tôi", 0.7).await;
    let url = format!("/api/v1/loras/{}", r.lora_id);
    let k2 = format!("{}-e2", r.lora_key);
    assert_eq!(
        run_files_job(&app, &r, published_result(&k2)).await.status,
        StatusCode::OK
    );
    // The Style still uses it.
    let blocked = app.call("DELETE", &url, &r.carol, None).await;
    assert_eq!(blocked.status, StatusCode::CONFLICT);
    assert!(
        blocked.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Còn Style dùng LoRA này: Lo-fi của tôi")
    );
    // Detach the Style (not just deactivate it), then delete.
    let sid: String = sqlx::query_scalar("SELECT id FROM style_presets WHERE lora_id = ?")
        .bind(&r.lora_id)
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    let off = app
        .call(
            "PATCH",
            &format!("/api/v1/presets/{sid}"),
            &r.admin,
            Some(json!({"kind": "genre", "lora_id": null, "lora_scale": null})),
        )
        .await;
    assert_eq!(
        off.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&off.bytes)
    );
    // Work still running on the node blocks it.
    sqlx::query("UPDATE jobs SET status = 'running' WHERE type = 'eval_lora'")
        .execute(&app.state.db)
        .await
        .unwrap();
    let busy = app.call("DELETE", &url, &r.carol, None).await;
    assert_eq!(busy.status, StatusCode::CONFLICT);
    assert!(
        busy.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("LoRA đang có việc chạy trên máy")
    );
    sqlx::query("UPDATE jobs SET status = 'queued' WHERE type = 'eval_lora'")
        .execute(&app.state.db)
        .await
        .unwrap();
    let gone = app.call("DELETE", &url, &r.carol, None).await;
    assert_eq!(gone.status, StatusCode::NO_CONTENT);
    // The waiting listening job is cancelled.
    let eval: String = sqlx::query_scalar("SELECT status FROM jobs WHERE type = 'eval_lora'")
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    assert_eq!(eval, "cancelled");
    let status: String = sqlx::query_scalar("SELECT status FROM loras WHERE id = ?")
        .bind(&r.lora_id)
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    assert_eq!(status, "archived");
    let jobs = files_jobs(&app).await;
    let del = jobs.iter().find(|j| j.3["op"] == "delete").unwrap();
    let ids: Vec<&str> = del.3["checkpoints"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&r.cps[0].as_str()) && ids.contains(&r.cps[1].as_str()));
    assert!(cp_row(&app, &r.cps[1]).await.2.is_some());
    // Unknown LoRA.
    let missing = app
        .call("DELETE", "/api/v1/loras/none", &r.carol, None)
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}

// ---------- LoRA quick try ----------

async fn try_lora(app: &TestApp, r: &Reviewed, cp: &str, prompt: &str, scale: f64) -> Reply {
    app.call(
        "POST",
        &format!("/api/v1/loras/{}/try", r.lora_id),
        &r.carol,
        Some(json!({"checkpoint_id": cp, "prompt": prompt, "lora_scale": scale})),
    )
    .await
}

async fn set_node_path(app: &TestApp, cp: &str, path: &str) {
    sqlx::query("UPDATE lora_checkpoints SET node_path = ? WHERE id = ?")
        .bind(path)
        .bind(cp)
        .execute(&app.state.db)
        .await
        .unwrap();
}

#[tokio::test]
async fn quick_try_creates_pinned_generate_job() {
    let app = TestApp::new().await;
    let r = reviewed_lora(&app).await;
    set_node_path(&app, &r.cps[1], "/mnt/train/secret/epoch_2").await;
    let res = try_lora(&app, &r, &r.cps[1], "  rainy window  ", 0.6).await;
    assert_eq!(
        res.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&res.bytes)
    );
    let job = res.json();
    assert_eq!(job["type"], "generate");
    assert_eq!(
        job["title"],
        format!("Thử nhanh · {} · Epoch 2", {
            let n: String = sqlx::query_scalar("SELECT name FROM loras WHERE id = ?")
                .bind(&r.lora_id)
                .fetch_one(&app.state.db)
                .await
                .unwrap();
            n
        })
    );
    let id = job["id"].as_str().unwrap();
    let (payload, lora_id, owner): (String, Option<String>, String) =
        sqlx::query_as("SELECT payload, lora_id, owner_id FROM jobs WHERE id = ?")
            .bind(id)
            .fetch_one(&app.state.db)
            .await
            .unwrap();
    let payload: Value = serde_json::from_str(&payload).unwrap();
    assert!(lora_id.is_none());
    assert_eq!(payload["node_id"], r.node_id);
    assert_eq!(
        payload["lora"],
        json!({"key": format!("{}-e2", r.lora_key), "scale": 0.6, "path": "/mnt/train/secret/epoch_2"})
    );
    // 60 s pulled into the catalog genre's range (the same rule as the listening renders).
    let eval_duration: i64 = serde_json::from_str::<Value>(
        &sqlx::query_scalar::<_, String>("SELECT payload FROM jobs WHERE type = 'eval_lora'")
            .fetch_one(&app.state.db)
            .await
            .unwrap(),
    )
    .unwrap()["duration"]
        .as_i64()
        .unwrap();
    assert_eq!(payload["duration"], eval_duration);
    assert!(payload["prompt"].as_str().unwrap().contains("rainy window"));
    let owner_name: String = sqlx::query_scalar("SELECT username FROM users WHERE id = ?")
        .bind(owner)
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    assert_eq!(owner_name, "carol");

    // Another node never takes it; the training node does.
    let admin = &r.admin;
    let (_, other) = app
        .node(
            admin,
            "other-node",
            json!([{"engine": "mock", "key": "acestep-v15-xl-sft", "capabilities": ["generate"]}]),
        )
        .await;
    assert_eq!(app.claim(&other).await.status, StatusCode::NO_CONTENT);
    let claimed = app.claim(&r.s.token).await;
    assert_eq!(claimed.status, StatusCode::OK);
    let claimed = claimed.json();
    let (ty, jid) = (claimed["type"].as_str().unwrap(), claimed["id"].as_str());
    assert_eq!(ty, "generate");
    assert_eq!(jid, Some(id));
}

#[tokio::test]
async fn quick_try_uses_base_key_after_a_publish() {
    let app = TestApp::new().await;
    let r = reviewed_lora(&app).await;
    publish_and_complete(&app, &r, 1, "Lo-fi").await;
    let current: String = sqlx::query_scalar("SELECT key FROM loras WHERE id = ?")
        .bind(&r.lora_id)
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    assert!(
        current.ends_with("-e2") || current.ends_with("-e1"),
        "{current}"
    );
    let res = try_lora(&app, &r, &r.cps[0], "x", 0.7).await;
    assert_eq!(res.status, StatusCode::CREATED);
    let payload: String = sqlx::query_scalar("SELECT payload FROM jobs WHERE id = ?")
        .bind(res.json()["id"].as_str().unwrap())
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    let payload: Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(payload["lora"]["key"], format!("{}-e1", r.lora_key));
}

#[tokio::test]
async fn quick_try_rejects_deleted_checkpoint() {
    let app = TestApp::new().await;
    let r = reviewed_lora(&app).await;
    let gone = "Bản này không còn trên máy".replace("Bản này", "Bản lưu này");
    sqlx::query("UPDATE lora_checkpoints SET status = 'deleted' WHERE id = ?")
        .bind(&r.cps[0])
        .execute(&app.state.db)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE lora_checkpoints SET delete_requested_at = '2026-01-01T00:00:00Z' WHERE id = ?",
    )
    .bind(&r.cps[1])
    .execute(&app.state.db)
    .await
    .unwrap();
    for cp in &r.cps {
        let res = try_lora(&app, &r, cp, "x", 0.7).await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            res.json()["error"]["message"]
                .as_str()
                .unwrap()
                .contains(&gone)
        );
    }
    // Foreign checkpoint id.
    let res = try_lora(&app, &r, "nope", "x", 0.7).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    let jobs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE type = 'generate'")
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    assert_eq!(jobs, 0);
}

#[tokio::test]
async fn quick_try_validates_input_and_archived_lora() {
    let app = TestApp::new().await;
    let r = reviewed_lora(&app).await;
    let long = "a".repeat(501);
    for prompt in ["", "   ", long.as_str()] {
        let res = try_lora(&app, &r, &r.cps[0], prompt, 0.7).await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            res.json()["error"]["message"]
                .as_str()
                .unwrap()
                .contains("Câu lệnh phải có 1–500 ký tự")
        );
    }
    assert_eq!(
        try_lora(&app, &r, &r.cps[0], &"a".repeat(500), 0.7)
            .await
            .status,
        StatusCode::CREATED
    );
    for scale in [-0.1, 1.1] {
        assert_eq!(
            try_lora(&app, &r, &r.cps[0], "x", scale).await.status,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    // Members cannot.
    let member = app.login("bob").await;
    let res = app
        .call(
            "POST",
            &format!("/api/v1/loras/{}/try", r.lora_id),
            &member,
            Some(json!({"checkpoint_id": r.cps[0], "prompt": "x", "lora_scale": 0.5})),
        )
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    sqlx::query("UPDATE loras SET status = 'archived' WHERE id = ?")
        .bind(&r.lora_id)
        .execute(&app.state.db)
        .await
        .unwrap();
    let res = try_lora(&app, &r, &r.cps[0], "x", 0.7).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        res.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("LoRA đã bị xóa")
    );
    let res = app
        .call(
            "POST",
            "/api/v1/loras/nope/try",
            &r.carol,
            Some(json!({"checkpoint_id": "x", "prompt": "x", "lora_scale": 0.5})),
        )
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn quick_try_job_detail_hides_path() {
    let app = TestApp::new().await;
    let r = reviewed_lora(&app).await;
    let secret = "/mnt/train/secret/epoch_1";
    set_node_path(&app, &r.cps[0], secret).await;
    let res = try_lora(&app, &r, &r.cps[0], "x", 0.7).await;
    assert_eq!(res.status, StatusCode::CREATED);
    let created = String::from_utf8_lossy(&res.bytes).to_string();
    let id = res.json()["id"].as_str().unwrap().to_string();
    let detail = app
        .call("GET", &format!("/api/v1/jobs/{id}"), &r.carol, None)
        .await;
    let list = app.call("GET", "/api/v1/jobs", &r.carol, None).await;
    for body in [
        created,
        String::from_utf8_lossy(&detail.bytes).to_string(),
        String::from_utf8_lossy(&list.bytes).to_string(),
    ] {
        assert!(!body.contains(secret), "{body}");
        assert!(!body.contains("node_path"));
    }
    assert!(String::from_utf8_lossy(&detail.bytes).contains(&format!("\"{id}\"")));
}

// ---------- Reference Audio: prompt written by a captioner (describe_input) ----------

/// A node that runs `describe_input` with the given captioner keys. Returns its token.
async fn describe_node(app: &TestApp, admin: &str, name: &str, captioners: &[&str]) -> String {
    let created = app
        .call("POST", "/api/v1/nodes", admin, Some(json!({"name": name})))
        .await;
    let token = created.json()["token"].as_str().unwrap().to_string();
    let mut models =
        vec![json!({"engine": "mock", "key": "mock-v1", "capabilities": ["generate"]})];
    for key in captioners {
        models.push(
            json!({"engine": "captioner", "key": key, "name": key, "capabilities": ["caption"]}),
        );
    }
    let body = json!({
        "protocol_version": 1, "worker_version": "test", "fresh_start": true,
        "job_types": ["generate", "describe_input"],
        "engines": [{"name": "mock", "version": "1", "capabilities": ["generate"]}],
        "models": models, "loras": [], "gpu": null,
    });
    let r = app
        .call("POST", "/api/worker/v1/register", &token, Some(body))
        .await;
    assert_eq!(r.status, StatusCode::OK);
    token
}

#[tokio::test]
async fn reference_prompt_job_prefers_acestep_captioner_and_reuses_a_live_job() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let alice = app.login("alice").await;
    let bob = app.login("bob").await;
    let input = app.upload_input(&alice, "ref.wav", WAV).await.json()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let url = format!("/api/v1/audio-inputs/{input}/prompt-jobs");

    // No captioner on any node yet.
    let r = app.call("POST", &url, &alice, None).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(String::from_utf8_lossy(&r.bytes).contains("Chưa có model mô tả"));

    let token = describe_node(&app, &admin, "desc-node", &["cap-x", "acestep-captioner"]).await;
    // Someone else's file is not found.
    assert_eq!(
        app.call("POST", &url, &bob, None).await.status,
        StatusCode::NOT_FOUND
    );

    let r = app.call("POST", &url, &alice, None).await;
    assert_eq!(r.status, StatusCode::CREATED);
    let created = r.json();
    assert_eq!(created["model_key"], "acestep-captioner");
    let job_id = created["job_id"].as_str().unwrap().to_string();
    // A second call while the job is live returns the same job.
    let again = app.call("POST", &url, &alice, None).await;
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(again.json()["job_id"], job_id.as_str());

    // It is claimed before an older queued generation (higher priority).
    let model_id = app.model_id(&alice, "mock-v1").await;
    app.generate(&alice, request_body(&model_id)).await;
    let claimed = app.claim(&token).await.json();
    assert_eq!(claimed["id"], job_id.as_str());
    assert_eq!(claimed["type"], "describe_input");
    assert_eq!(claimed["payload"]["model"]["key"], "acestep-captioner");
    assert_eq!(
        claimed["payload"]["inputs"]["audio"]["artifact_id"],
        input.as_str()
    );
}

#[tokio::test]
async fn reference_prompt_result_is_checked_and_normalized() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let alice = app.login("alice").await;
    let token = describe_node(&app, &admin, "desc-node", &["cap-x"]).await;
    let input = app.upload_input(&alice, "ref.wav", WAV).await.json()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let url = format!("/api/v1/audio-inputs/{input}/prompt-jobs");
    let created = app.call("POST", &url, &alice, None).await.json();
    assert_eq!(created["model_key"], "cap-x"); // the only captioner there is
    let job_id = created["job_id"].as_str().unwrap().to_string();
    let lease = app.claim(&token).await.json()["lease_token"]
        .as_str()
        .unwrap()
        .to_string();
    let complete = format!("/api/worker/v1/jobs/{job_id}/complete");

    let empty = json!({"lease_token": lease, "result": {"caption": "  "}});
    let r = app.call("POST", &complete, &token, Some(empty)).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);

    let good = json!({"lease_token": lease, "result": {
        "caption": " A chill lo-fi track. ", "mood": "chill", "instruments": "rhodes", "vocals": "Instrumental",
        "source_ref": "cap-x@1", "tempo_bpm": 82.4, "key": "am"}});
    let r = app.call("POST", &complete, &token, Some(good)).await;
    assert_eq!(
        r.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&r.bytes)
    );
    let job = app
        .call("GET", &format!("/api/v1/jobs/{job_id}"), &alice, None)
        .await
        .json();
    assert_eq!(job["status"], "completed");
    assert_eq!(job["result"]["caption"], "A chill lo-fi track.");
    assert_eq!(job["result"]["key"], "A minor");
    assert_eq!(job["result"]["tempo_bpm"].as_f64(), Some(82.4));

    // Finished: asking again makes a new job ("Viết lại"), and odd tempo/key become null.
    let r = app.call("POST", &url, &alice, None).await;
    assert_eq!(r.status, StatusCode::CREATED);
    let job2 = r.json()["job_id"].as_str().unwrap().to_string();
    assert_ne!(job2, job_id);
    let lease = app.claim(&token).await.json()["lease_token"]
        .as_str()
        .unwrap()
        .to_string();
    let long = "A warm lo-fi groove. ".repeat(150); // 3150 chars: cut to the 2000-char prompt limit
    let odd = json!({"lease_token": lease, "result": {"caption": long, "tempo_bpm": 900, "key": "A dorian"}});
    let r = app
        .call(
            "POST",
            &format!("/api/worker/v1/jobs/{job2}/complete"),
            &token,
            Some(odd),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK);
    let job = app
        .call("GET", &format!("/api/v1/jobs/{job2}"), &alice, None)
        .await
        .json();
    assert_eq!(job["result"]["tempo_bpm"], Value::Null);
    assert_eq!(job["result"]["key"], Value::Null);
    let caption = job["result"]["caption"].as_str().unwrap();
    assert!(
        caption.chars().count() <= 2000 && caption.ends_with("groove."),
        "{}",
        caption.len()
    );
}

#[tokio::test]
async fn a_long_prompt_reaches_the_engine_within_what_ace_step_reads() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let alice = app.login("alice").await;
    let (_, token) = app.node(&admin, "render-node-01", mock_models()).await;
    let model_id = app.model_id(&alice, "mock-v1").await;
    let mut body = request_body(&model_id);
    body["prompt"] = json!(
        "A slow warm lo-fi groove with dusty drums. "
            .repeat(40)
            .trim()
    );
    body["instruments"] = json!(["piano"]);
    let r = app.generate(&alice, body).await;
    assert_eq!(
        r.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&r.bytes)
    );
    let claimed = app.claim(&token).await.json();
    let prompt = claimed["payload"]["prompt"].as_str().unwrap();
    assert!(prompt.chars().count() <= 850, "{}", prompt.chars().count());
    assert!(prompt.contains("piano"), "{prompt}");
}

#[tokio::test]
async fn reference_prompt_job_only_goes_to_a_node_with_that_captioner() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let alice = app.login("alice").await;
    let other = describe_node(&app, &admin, "other-node", &["cap-x"]).await;
    let right = describe_node(&app, &admin, "right-node", &["acestep-captioner"]).await;
    let input = app.upload_input(&alice, "ref.wav", WAV).await.json()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let created = app
        .call(
            "POST",
            &format!("/api/v1/audio-inputs/{input}/prompt-jobs"),
            &alice,
            None,
        )
        .await
        .json();
    assert_eq!(created["model_key"], "acestep-captioner");
    assert_eq!(app.claim(&other).await.status, StatusCode::NO_CONTENT);
    assert_eq!(app.claim(&right).await.json()["id"], created["job_id"]);
}

// ---------- params.thinking (LM plans the song, D36) ----------

#[tokio::test]
async fn thinking_needs_a_model_with_the_thinking_capability() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let alice = app.login("alice").await;
    let models = json!([
        {"engine": "acestep", "key": "acestep-v15-xl-sft", "name": "XL", "capabilities": ["generate", "lora", "thinking"]},
        {"engine": "mock", "key": "mock-v1", "name": "Mock", "capabilities": ["generate"]},
    ]);
    let (_, token) = app.node(&admin, "render-node-01", models.clone()).await;
    let xl = app.model_id(&alice, "acestep-v15-xl-sft").await;
    let mock = app.model_id(&alice, "mock-v1").await;
    let body_for = |model: &str, params: Value| json!({"prompt": "Lo-fi", "model_id": model, "duration": 30, "params": params});

    // generate_thinking_without_capability_is_422
    let r = app
        .generate(&alice, body_for(&mock, json!({"thinking": true})))
        .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        r.json().to_string().contains("LM chưa bật trên máy render"),
        "{}",
        r.json()
    );
    // generate_thinking_non_bool_is_422
    for bad in [json!("true"), json!(1)] {
        let r = app
            .generate(&alice, body_for(&xl, json!({"thinking": bad})))
            .await;
        assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    }
    // false needs no LM
    assert_eq!(
        app.generate(&alice, body_for(&mock, json!({"thinking": false})))
            .await
            .status,
        StatusCode::CREATED
    );
    // generate_thinking_with_capability_queues_job_with_param
    assert_eq!(
        app.generate(&alice, body_for(&xl, json!({"thinking": true})))
            .await
            .status,
        StatusCode::CREATED
    );
    let mut seen = vec![];
    for _ in 0..2 {
        let claim = app.claim(&token).await.json();
        seen.push(claim["payload"]["params"]["thinking"].clone());
        app.call(
            "POST",
            &format!("/api/worker/v1/jobs/{}/fail", claim["id"].as_str().unwrap()),
            &token,
            Some(json!({"lease_token": claim["lease_token"], "error": "skip"})),
        )
        .await;
    }
    assert!(
        seen.contains(&json!(true)) && seen.contains(&json!(false)),
        "{seen:?}"
    );

    // preset_params_thinking_non_bool_is_422
    let genres = app.call("GET", "/api/v1/genres", &alice, None).await.json();
    let genre_id = genres[0]["id"].as_str().unwrap().to_string();
    let bad = app
        .call(
            "POST",
            "/api/v1/presets",
            &admin,
            Some(json!({"genre_id": genre_id, "name": "LM", "params": {"thinking": "yes"}})),
        )
        .await;
    assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY);
    let preset = app
        .call(
            "POST",
            "/api/v1/presets",
            &admin,
            Some(json!({"genre_id": genre_id, "name": "LM", "model_id": xl, "params": {"thinking": true}})),
        )
        .await;
    assert_eq!(preset.status, StatusCode::CREATED);
    let preset_id = preset.json()["id"].as_str().unwrap().to_string();

    // generate_from_preset_with_thinking_after_lm_removed_is_422: the LM is switched off on the node.
    let no_lm = json!([
        {"engine": "acestep", "key": "acestep-v15-xl-sft", "name": "XL", "capabilities": ["generate", "lora"]},
        {"engine": "mock", "key": "mock-v1", "name": "Mock", "capabilities": ["generate"]},
    ]);
    assert_eq!(
        app.register(&token, no_lm, false).await.status,
        StatusCode::OK
    );
    let r = app
        .generate(
            &alice,
            json!({"prompt": "Lo-fi", "preset_id": preset_id, "duration": 30}),
        )
        .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(r.json().to_string().contains("LM chưa bật trên máy render"));
}

// ---------- metas ACE-Step used are pinned on the track (D36 follow-up) ----------

async fn finish_generate(app: &TestApp, token: &str, metas: Value) -> String {
    let claim = app.claim(token).await.json();
    let job_id = claim["id"].as_str().unwrap().to_string();
    let lease = claim["lease_token"].as_str().unwrap().to_string();
    let sha = sha256_hex(WAV);
    let up = app
        .upload(
            token,
            &job_id,
            &lease,
            "audio_master",
            "master.wav",
            WAV,
            &sha,
        )
        .await;
    assert_eq!(up.status, StatusCode::OK);
    let done = app
        .call(
            "POST",
            &format!("/api/worker/v1/jobs/{job_id}/complete"),
            token,
            Some(json!({"lease_token": lease, "result": {"seed": 7, "duration": 30.0, "sample_rate": 48000,
                "engine": "acestep", "model_key": "acestep-v15-xl-sft",
                "engine_info": {"dit_model": "acestep-v15-xl-sft", "metas": metas}}})),
        )
        .await;
    assert_eq!(done.status, StatusCode::OK);
    job_id
}

#[tokio::test]
async fn track_pins_the_metas_ace_step_used_when_the_request_left_them_blank() {
    let app = TestApp::new().await;
    let admin = app.login("admin").await;
    let alice = app.login("alice").await;
    let models = json!([{"engine": "acestep", "key": "acestep-v15-xl-sft", "name": "XL", "capabilities": ["generate"]}]);
    let (_, token) = app.node(&admin, "render-node-01", models).await;
    let xl = app.model_id(&alice, "acestep-v15-xl-sft").await;

    // Blank BPM / Key / Time Signature: the track keeps what the LM chose.
    let r = app
        .generate(
            &alice,
            json!({"prompt": "Lo-fi", "model_id": xl, "duration": 30}),
        )
        .await;
    assert_eq!(r.status, StatusCode::CREATED);
    let job_id = finish_generate(
        &app,
        &token,
        json!({"bpm": 40, "keyscale": "c minor", "timesignature": "2"}),
    )
    .await;
    let track_id = app.job(&alice, &job_id).await["track_id"]
        .as_str()
        .unwrap()
        .to_string();
    let track = app
        .call("GET", &format!("/api/v1/tracks/{track_id}"), &alice, None)
        .await
        .json();
    assert_eq!(track["bpm"], 40);
    assert_eq!(track["key"], "C minor");
    assert_eq!(track["time_signature"], "2/4");

    // Values the user typed win over the metas; unusable metas stay empty.
    let r = app
        .generate(
            &alice,
            json!({"prompt": "Lo-fi", "model_id": xl, "duration": 30, "bpm": 80, "key": "A minor"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::CREATED);
    let job_id = finish_generate(
        &app,
        &token,
        json!({"bpm": 120, "keyscale": "F major", "timesignature": "N/A"}),
    )
    .await;
    let track_id = app.job(&alice, &job_id).await["track_id"]
        .as_str()
        .unwrap()
        .to_string();
    let track = app
        .call("GET", &format!("/api/v1/tracks/{track_id}"), &alice, None)
        .await
        .json();
    assert_eq!(track["bpm"], 80);
    assert_eq!(track["key"], "A minor");
    assert_eq!(track["time_signature"], Value::Null);
}
