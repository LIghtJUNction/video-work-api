//! HTTP smoke tests with a fake speech engine.

use std::fs;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tempfile::tempdir;
use tower::ServiceExt;
use video_work_api::config::{McpTokenSource, Settings};
use video_work_api::database::{Database, NewRenderJob};
use video_work_api::engine::FakeEngine;
use video_work_api::http::{build_router, AppState, LoginLimiter};
use video_work_api::model::ModelDownloadRegistry;
use video_work_api::passkeys::CeremonyStore;
use video_work_api::studio::Studio;
use video_work_api::subtitles::FakeSubtitles;
use video_work_api::translation::FakeTranslationEngine;

fn test_settings(root: &std::path::Path) -> Settings {
    Settings {
        data_dir: root.to_path_buf(),
        model_dir: root.join("model"),
        translation_model_dir: root.join("translation-model"),
        cosyvoice_root: root.join("source"),
        setup_token_file: root.join("setup-token"),
        host: "127.0.0.1".into(),
        port: 7860,
        ssl_certfile: None,
        ssl_keyfile: None,
        mcp_token: Some("mcp-secret-token".into()),
        mcp_token_file: root.join("mcp-token"),
        mcp_token_source: Some(McpTokenSource::Environment),
        funclip_root: None,
        video_input_dir: root.join("videos"),
        reference_input_dir: root.join("references"),
        video_projects_dir: root.join("video-projects"),
        receipt_key_file: root.join("receipt.key"),
        subtitle_timeout_seconds: 30,
        translation_timeout_seconds: 30,
        xry_task_root: root.join("xry-tasks"),
        xry_source_root: root.join("xry-sources"),
        xry_renderer: root.join("render_variants.py"),
        xry_python: std::path::PathBuf::from("/usr/bin/python3")
            .canonicalize()
            .unwrap(),
        render_timeout_seconds: 30,
        video_project_renderer: root.join("video_project_render.py"),
        video_project_python: std::path::PathBuf::from("/usr/bin/python3")
            .canonicalize()
            .unwrap(),
        video_project_render_timeout_seconds: 30,
        project_root: root.to_path_buf(),
    }
}

fn create_complete_model_assets(model_dir: &std::path::Path) {
    for relative in [
        "config.json",
        "cosyvoice3.yaml",
        "campplus.onnx",
        "speech_tokenizer_v3.onnx",
        "llm.pt",
        "flow.pt",
        "hift.pt",
        "CosyVoice-BlankEN/config.json",
        "CosyVoice-BlankEN/model.safetensors",
        "CosyVoice-BlankEN/tokenizer_config.json",
        "CosyVoice-BlankEN/merges.txt",
        "CosyVoice-BlankEN/vocab.json",
    ] {
        let path = model_dir.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"test").unwrap();
    }
}

async fn body_json(res: axum::response::Response) -> serde_json::Value {
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::json!({}))
}

async fn read_sse_until(body: &mut Body, marker: &str) -> String {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut received = String::new();
    loop {
        let frame = tokio::time::timeout_at(deadline, body.frame())
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for SSE marker {marker}"))
            .expect("SSE stream ended")
            .expect("SSE body failed");
        if let Ok(data) = frame.into_data() {
            received.push_str(std::str::from_utf8(&data).unwrap());
            if received.contains(marker) {
                return received;
            }
        }
    }
}

#[tokio::test]
async fn setup_login_and_status() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("static")).unwrap();
    fs::write(root.join("static/index.html"), b"<html></html>").unwrap();
    let settings = test_settings(root);
    settings.create_data_dirs().unwrap();
    create_complete_model_assets(&settings.model_dir);
    fs::write(&settings.setup_token_file, "setup-secret\n").unwrap();
    let db = Database::open(settings.database_path()).unwrap();
    let studio = Arc::new(Studio::new(
        settings,
        db,
        Arc::new(FakeEngine::new()),
        Arc::new(FakeSubtitles::default()),
        Arc::new(FakeTranslationEngine::new()),
    ));
    let app = build_router(AppState {
        studio: studio.clone(),
        limiter: Arc::new(LoginLimiter::new()),
        passkey_ceremonies: Arc::new(CeremonyStore::new()),
        model_downloads: Arc::new(ModelDownloadRegistry::new()),
    });

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/setup")
                .header("host", "testserver")
                .header("origin", "http://testserver")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"token":"setup-secret","password":"correct horse battery staple"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .header("host", "testserver")
                .header("origin", "http://testserver")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"password":"correct horse battery staple"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let cookie = res
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    let res = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/status")
                .header("cookie", cookie.split(';').next().unwrap())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let json = body_json(res).await;
    assert_eq!(json["product"], "video-work-api");
    assert_eq!(json["configured"], true);
    assert_eq!(json["authenticated"], true);
    assert_eq!(json["passkey_login_available"], false);
    assert_eq!(json["model_present"], true);
    assert_eq!(json["model_runtime_ready"], false);
    assert_eq!(json["model_ready"], false);
    assert_eq!(json["translation"]["model"], "google/madlad400-3b-mt");
    assert_eq!(json["models"]["voice"]["kind"], "voice");
    assert_eq!(json["models"]["translation"]["kind"], "translation");
    assert_eq!(json["models"]["translation"]["id"], "google/madlad400-3b-mt");
    assert_eq!(json["limits"]["max_translate_segments"], 200);
}

#[tokio::test]
async fn passkey_management_requires_auth_and_login_start_handles_empty_store() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("static")).unwrap();
    fs::write(root.join("static/index.html"), b"<html></html>").unwrap();
    let settings = test_settings(root);
    settings.create_data_dirs().unwrap();
    let db = Database::open(settings.database_path()).unwrap();
    let studio = Arc::new(Studio::new(
        settings,
        db,
        Arc::new(FakeEngine::new()),
        Arc::new(FakeSubtitles::default()),
        Arc::new(FakeTranslationEngine::new()),
    ));
    let app = build_router(AppState {
        studio: studio.clone(),
        limiter: Arc::new(LoginLimiter::new()),
        passkey_ceremonies: Arc::new(CeremonyStore::new()),
        model_downloads: Arc::new(ModelDownloadRegistry::new()),
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/auth/passkeys")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/model/download")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/model/download")
                .header("host", "localhost")
                .header("origin", "http://localhost")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/passkeys/login/start")
                .header("host", "127.0.0.1:7860")
                .header("origin", "http://127.0.0.1:7860")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = body_json(response).await;
    assert_eq!(json["error"]["code"], "webauthn_ip_origin_unsupported");
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("http://localhost:<port>"));

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/passkeys/login/start")
                .header("host", "[::1]:7860")
                .header("origin", "http://[::1]:7860")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = body_json(response).await;
    assert_eq!(json["error"]["code"], "webauthn_ip_origin_unsupported");

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/passkeys/register/start")
                .header("host", "localhost")
                .header("origin", "http://localhost")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"name":"Laptop"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/passkeys/login/start")
                .header("host", "localhost")
                .header("origin", "http://localhost")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let json = body_json(response).await;
    assert_eq!(json["error"]["code"], "no_passkeys");
}

#[tokio::test]
async fn mcp_requires_bearer() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("static")).unwrap();
    fs::write(root.join("static/index.html"), b"<html></html>").unwrap();
    let settings = test_settings(root);
    settings.create_data_dirs().unwrap();
    let db = Database::open(settings.database_path()).unwrap();
    let studio = Arc::new(Studio::new(
        settings,
        db,
        Arc::new(FakeEngine::new()),
        Arc::new(FakeSubtitles::default()),
        Arc::new(FakeTranslationEngine::new()),
    ));
    let app = build_router(AppState {
        studio: studio.clone(),
        limiter: Arc::new(LoginLimiter::new()),
        passkey_ceremonies: Arc::new(CeremonyStore::new()),
        model_downloads: Arc::new(ModelDownloadRegistry::new()),
    });

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp")
                .header("content-type", "application/json")
                .header("authorization", "Bearer mcp-secret-token")
                .body(Body::from(
                    r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let json = body_json(res).await;
    let tools = json["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], "video_editor");
}

#[tokio::test]
async fn editor_rest_requires_same_origin_session_and_enforces_revisions() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("static")).unwrap();
    fs::write(root.join("static/index.html"), b"<html></html>").unwrap();
    let settings = test_settings(root);
    settings.create_data_dirs().unwrap();
    let db = Database::open(settings.database_path()).unwrap();
    db.create_session(&video_work_api::security::token_hash("editor-session"))
        .unwrap();
    let studio = Arc::new(Studio::new(
        settings,
        db,
        Arc::new(FakeEngine::new()),
        Arc::new(FakeSubtitles::default()),
        Arc::new(FakeTranslationEngine::new()),
    ));
    let app = build_router(AppState {
        studio: Arc::clone(&studio),
        limiter: Arc::new(LoginLimiter::new()),
        passkey_ceremonies: Arc::new(CeremonyStore::new()),
        model_downloads: Arc::new(ModelDownloadRegistry::new()),
    });
    let create = serde_json::json!({
        "action": "create_project",
        "slug": "aurora-launch"
    })
    .to_string();

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/editor")
                .header("host", "testserver")
                .header("content-type", "application/json")
                .body(Body::from(create.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/editor")
                .header("host", "testserver")
                .header("origin", "http://testserver")
                .header("content-type", "application/json")
                .body(Body::from(create.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/editor")
                .header("host", "testserver")
                .header("origin", "http://testserver")
                .header("cookie", "vwa_session=editor-session")
                .header("content-type", "application/json")
                .body(Body::from(create))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_json(response).await["revision"], 1);

    let write = serde_json::json!({
        "action": "write_file",
        "project": "aurora-launch",
        "path": "project.vpe",
        "content": "project \"Aurora Launch\" {\n  canvas 1080x1920 @ 30fps\n  source main = \"assets/source.mp4\"\n  timeline { track main { clip main source 00:00:00.000..00:00:01.000 at 00:00:00.000 } }\n}\n",
        "expected_revision": 1
    })
    .to_string();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/editor")
                .header("host", "testserver")
                .header("origin", "http://testserver")
                .header("cookie", "vwa_session=editor-session")
                .header("content-type", "application/json")
                .body(Body::from(write.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_json(response).await["revision"], 2);

    fs::write(
        &studio.settings.video_project_renderer,
        include_bytes!("../scripts/video_project_render.py"),
    )
    .unwrap();
    let status = std::process::Command::new("/usr/bin/ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:size=160x90:rate=10",
            "-t",
            "1.1",
            "-c:v",
            "libx264",
            "-an",
            "-y",
        ])
        .arg(
            studio
                .settings
                .video_projects_dir
                .join("aurora-launch/assets/source.mp4"),
        )
        .status()
        .unwrap();
    assert!(status.success());
    for action in ["validate", "export"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/editor")
                    .header("host", "testserver")
                    .header("origin", "http://testserver")
                    .header("cookie", "vwa_session=editor-session")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"action": action, "project": "aurora-launch"})
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = body_json(response).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["revision"], 2);
        if action == "export" {
            assert_eq!(body["render"]["job"]["kind"], "video_project");
            assert_eq!(body["render"]["job"]["project_id"], "aurora-launch");
            assert_eq!(body["render"]["job"]["project_revision"], 2);
            assert_eq!(
                body["render"]["job"]["document_sha"],
                body["document_sha256"]
            );
        }
    }

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/editor")
                .header("host", "testserver")
                .header("origin", "http://testserver")
                .header("cookie", "vwa_session=editor-session")
                .header("content-type", "application/json")
                .body(Body::from(write))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        body_json(response).await["error"]["code"],
        "editor_conflict"
    );

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/renders")
                .header("host", "testserver")
                .header("origin", "http://testserver")
                .header("cookie", "vwa_session=editor-session")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"task_dir":"group/batch","subject_id":"S01"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn editor_events_require_session_and_origin_and_emit_sanitized_changes() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("static")).unwrap();
    fs::write(root.join("static/index.html"), b"<html></html>").unwrap();
    let settings = test_settings(root);
    settings.create_data_dirs().unwrap();
    let db = Database::open(settings.database_path()).unwrap();
    db.create_session(&video_work_api::security::token_hash("event-session"))
        .unwrap();
    let studio = Arc::new(Studio::new(
        settings,
        db,
        Arc::new(FakeEngine::new()),
        Arc::new(FakeSubtitles::default()),
        Arc::new(FakeTranslationEngine::new()),
    ));
    let app = build_router(AppState {
        studio: Arc::clone(&studio),
        limiter: Arc::new(LoginLimiter::new()),
        passkey_ceremonies: Arc::new(CeremonyStore::new()),
        model_downloads: Arc::new(ModelDownloadRegistry::new()),
    });
    let create = serde_json::json!({
        "action": "create_project",
        "slug": "event-project"
    })
    .to_string();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/editor")
                .header("host", "testserver")
                .header("origin", "http://testserver")
                .header("cookie", "vwa_session=event-session")
                .header("content-type", "application/json")
                .body(Body::from(create))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    for (cookie, origin, expected) in [
        (None, "http://testserver", StatusCode::UNAUTHORIZED),
        (
            Some("vwa_session=event-session"),
            "http://elsewhere",
            StatusCode::FORBIDDEN,
        ),
    ] {
        let mut builder = Request::builder()
            .uri("/api/editor/events")
            .header("host", "testserver")
            .header("origin", origin);
        if let Some(cookie) = cookie {
            builder = builder.header("cookie", cookie);
        }
        let response = app
            .clone()
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/editor/events")
                .header("host", "testserver")
                .header("cookie", "vwa_session=event-session")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/editor/events")
                .header("host", "testserver")
                .header("sec-fetch-site", "same-origin")
                .header("cookie", "vwa_session=event-session")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    drop(response);
    for (uri, authorization, expected) in [
        (
            "/api/editor/events?token=forbidden",
            None,
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "/api/editor/events",
            Some("Bearer forbidden"),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
    ] {
        let mut builder = Request::builder()
            .uri(uri)
            .header("host", "testserver")
            .header("origin", "http://testserver")
            .header("cookie", "vwa_session=event-session");
        if let Some(authorization) = authorization {
            builder = builder.header("authorization", authorization);
        }
        let response = app
            .clone()
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap();
        if response.status() != expected {
            let status = response.status();
            let body = response.into_body().collect().await.unwrap().to_bytes();
            panic!(
                "{uri} authorization={authorization:?}: expected {expected}, got {status}: {}",
                String::from_utf8_lossy(&body)
            );
        }
    }

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/editor/events")
                .header("host", "testserver")
                .header("origin", "http://testserver")
                .header("cookie", "vwa_session=event-session")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["cache-control"],
        "no-store, no-cache, must-revalidate"
    );
    assert!(response.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/event-stream"));
    let mut events = response.into_body();
    let initial = read_sse_until(&mut events, "event: snapshot").await;
    assert!(initial.contains("event-project"));
    assert!(!initial.contains(&root.to_string_lossy().to_string()));
    for private in [
        "render_key",
        "task_dir",
        "log_path",
        "snapshot_dir",
        "renderer_hash",
        "publication_intent",
        "recovery_blocked",
    ] {
        assert!(!initial.contains(private));
    }

    let write = serde_json::json!({
        "action": "write_file",
        "project": "event-project",
        "path": "project.vpe",
        "content": "project \"Event Project\" {\n  canvas 1080x1920 @ 30fps\n}\n",
        "expected_revision": 1
    })
    .to_string();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/editor")
                .header("host", "testserver")
                .header("origin", "http://testserver")
                .header("cookie", "vwa_session=event-session")
                .header("content-type", "application/json")
                .body(Body::from(write))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let project_event = read_sse_until(&mut events, "event: project_changed").await;
    assert!(project_event.contains("\"revision\":2"));

    studio
        .database
        .insert_or_get_render_job(NewRenderJob {
            id: "event-job",
            render_key: "event-render-key",
            task_dir: "/private/task",
            subject_id: "subject",
            encoder_profile: "formal-auto",
            log_path: "/private/render.log",
            snapshot_dir: "/private/snapshot",
            snapshot_hash: "snapshot-hash",
            renderer_hash: "renderer-hash",
        })
        .unwrap();
    let job_event = read_sse_until(&mut events, "event: job_changed").await;
    assert!(job_event.contains("\"id\":\"event-job\""));
    assert!(job_event.contains("\"status\":\"queued\""));
    assert!(!job_event.contains("/private/"));
    assert!(!job_event.contains("event-render-key"));
}

#[tokio::test]
async fn editor_mcp_requires_bearer_and_dispatches_single_tool() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("static")).unwrap();
    fs::write(root.join("static/index.html"), b"<html></html>").unwrap();
    let settings = test_settings(root);
    settings.create_data_dirs().unwrap();
    let studio = Arc::new(Studio::new(
        settings.clone(),
        Database::open(settings.database_path()).unwrap(),
        Arc::new(FakeEngine::new()),
        Arc::new(FakeSubtitles::default()),
        Arc::new(FakeTranslationEngine::new()),
    ));
    let app = build_router(AppState {
        studio,
        limiter: Arc::new(LoginLimiter::new()),
        passkey_ceremonies: Arc::new(CeremonyStore::new()),
        model_downloads: Arc::new(ModelDownloadRegistry::new()),
    });
    let call = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 7,
        "method": "tools/call",
        "params": {
            "name": "video_editor",
            "arguments": {
                "action": "create_project",
                "slug": "mcp-project"
            }
        }
    })
    .to_string();

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp")
                .header("content-type", "application/json")
                .body(Body::from(call.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp")
                .header("authorization", "Bearer mcp-secret-token")
                .header("content-type", "application/json")
                .body(Body::from(call))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    assert_eq!(
        json["result"]["structuredContent"]["project"],
        "mcp-project"
    );
    assert_eq!(json["result"]["structuredContent"]["revision"], 1);

    let migrated = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 8,
        "method": "tools/call",
        "params": {
            "name": "video_editor",
            "arguments": { "action": "get_status" }
        }
    })
    .to_string();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp")
                .header("authorization", "Bearer mcp-secret-token")
                .header("content-type", "application/json")
                .body(Body::from(migrated))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        body_json(response).await["result"]["structuredContent"]["service"],
        "Video Work API"
    );

    let legacy = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 9,
        "method": "tools/call",
        "params": {
            "name": "get_status",
            "arguments": {}
        }
    })
    .to_string();
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp")
                .header("authorization", "Bearer mcp-secret-token")
                .header("content-type", "application/json")
                .body(Body::from(legacy))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_json(response).await["error"]["code"], -32601);
}

#[tokio::test]
async fn mcp_token_requires_same_origin_admin_session_and_disables_caching() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("static")).unwrap();
    fs::write(root.join("static/index.html"), b"<html></html>").unwrap();
    let settings = test_settings(root);
    settings.create_data_dirs().unwrap();
    let db = Database::open(settings.database_path()).unwrap();
    db.set_admin(&video_work_api::security::hash_password("correct horse battery staple").unwrap())
        .unwrap();
    let studio = Arc::new(Studio::new(
        settings,
        db,
        Arc::new(FakeEngine::new()),
        Arc::new(FakeSubtitles::default()),
        Arc::new(FakeTranslationEngine::new()),
    ));
    let app = build_router(AppState {
        studio,
        limiter: Arc::new(LoginLimiter::new()),
        passkey_ceremonies: Arc::new(CeremonyStore::new()),
        model_downloads: Arc::new(ModelDownloadRegistry::new()),
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/mcp-token")
                .header("host", "testserver")
                .header("origin", "http://testserver")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .header("host", "testserver")
                .header("origin", "http://testserver")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"password":"correct horse battery staple"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = response
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/mcp-token")
                .header("host", "testserver")
                .header("origin", "http://evil.example")
                .header("cookie", &cookie)
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/mcp-token")
                .header("host", "testserver")
                .header("origin", "http://testserver")
                .header("cookie", &cookie)
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("cache-control").unwrap(),
        "no-store, max-age=0, must-revalidate"
    );
    assert_eq!(response.headers().get("pragma").unwrap(), "no-cache");
    assert_eq!(response.headers().get("expires").unwrap(), "0");
    let json = body_json(response).await;
    assert!(json["token"]
        .as_str()
        .is_some_and(|value| value == "mcp-secret-token"));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let json = body_json(response).await;
    assert!(json["mcp"]["configured"].as_bool().unwrap());
    assert!(!json.to_string().contains("mcp-secret-token"));
}

#[tokio::test]
async fn mcp_token_endpoint_is_unavailable_when_not_configured() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("static")).unwrap();
    fs::write(root.join("static/index.html"), b"<html></html>").unwrap();
    let mut settings = test_settings(root);
    settings.mcp_token = None;
    settings.create_data_dirs().unwrap();
    let db = Database::open(settings.database_path()).unwrap();
    db.create_session(&video_work_api::security::token_hash("test-session"))
        .unwrap();
    let studio = Arc::new(Studio::new(
        settings,
        db,
        Arc::new(FakeEngine::new()),
        Arc::new(FakeSubtitles::default()),
        Arc::new(FakeTranslationEngine::new()),
    ));
    let app = build_router(AppState {
        studio,
        limiter: Arc::new(LoginLimiter::new()),
        passkey_ceremonies: Arc::new(CeremonyStore::new()),
        model_downloads: Arc::new(ModelDownloadRegistry::new()),
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/mcp-token")
                .header("host", "testserver")
                .header("origin", "http://testserver")
                .header("cookie", "vwa_session=test-session")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let json = body_json(response).await;
    assert_eq!(json["error"]["code"], "mcp_not_configured");
}

#[tokio::test]
async fn rejects_cross_origin_mutations() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("static")).unwrap();
    fs::write(root.join("static/index.html"), b"<html></html>").unwrap();
    let settings = test_settings(root);
    settings.create_data_dirs().unwrap();
    let db = Database::open(settings.database_path()).unwrap();
    let studio = Arc::new(Studio::new(
        settings,
        db,
        Arc::new(FakeEngine::new()),
        Arc::new(FakeSubtitles::default()),
        Arc::new(FakeTranslationEngine::new()),
    ));
    let app = build_router(AppState {
        studio,
        limiter: Arc::new(LoginLimiter::new()),
        passkey_ceremonies: Arc::new(CeremonyStore::new()),
        model_downloads: Arc::new(ModelDownloadRegistry::new()),
    });

    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .header("origin", "http://evil.example")
                .header("host", "testserver")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"password":"x"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn spoofed_forwarded_for_does_not_bypass_login_rate_limit() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("static")).unwrap();
    fs::write(root.join("static/index.html"), b"<html></html>").unwrap();
    let settings = test_settings(root);
    settings.create_data_dirs().unwrap();
    let db = Database::open(settings.database_path()).unwrap();
    let studio = Arc::new(Studio::new(
        settings,
        db,
        Arc::new(FakeEngine::new()),
        Arc::new(FakeSubtitles::default()),
        Arc::new(FakeTranslationEngine::new()),
    ));
    let app = build_router(AppState {
        studio,
        limiter: Arc::new(LoginLimiter::new()),
        passkey_ceremonies: Arc::new(CeremonyStore::new()),
        model_downloads: Arc::new(ModelDownloadRegistry::new()),
    });

    for attempt in 0..9 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/login")
                    .header("host", "localhost")
                    .header("origin", "http://localhost")
                    .header("x-forwarded-for", format!("203.0.113.{attempt}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"password":"wrong"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let expected = if attempt < 8 {
            StatusCode::UNAUTHORIZED
        } else {
            StatusCode::TOO_MANY_REQUESTS
        };
        assert_eq!(response.status(), expected);
    }
}

#[tokio::test]
async fn subtitle_upload_requires_auth_extracts_and_cleans_up() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("static")).unwrap();
    fs::write(root.join("static/index.html"), b"<html></html>").unwrap();
    let settings = test_settings(root);
    settings.create_data_dirs().unwrap();
    let db = Database::open(settings.database_path()).unwrap();
    db.set_admin(&video_work_api::security::hash_password("correct horse battery staple").unwrap())
        .unwrap();
    let studio = Arc::new(Studio::new(
        settings.clone(),
        db,
        Arc::new(FakeEngine::new()),
        Arc::new(FakeSubtitles::default()),
        Arc::new(FakeTranslationEngine::new()),
    ));
    let app = build_router(AppState {
        studio,
        limiter: Arc::new(LoginLimiter::new()),
        passkey_ceremonies: Arc::new(CeremonyStore::new()),
        model_downloads: Arc::new(ModelDownloadRegistry::new()),
    });

    let boundary = "vwa-test-boundary";
    let multipart = |filename: &str| -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"video\"; filename=\"{filename}\"\r\nContent-Type: video/mp4\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(b"fake video bytes");
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        body
    };
    let content_type = format!("multipart/form-data; boundary={boundary}");

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/videos/subtitles/upload")
                .header("host", "testserver")
                .header("origin", "http://testserver")
                .header("content-type", &content_type)
                .body(Body::from(multipart("clip.mp4")))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .header("host", "testserver")
                .header("origin", "http://testserver")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"password":"correct horse battery staple"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let cookie = res
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/videos/subtitles/upload")
                .header("host", "testserver")
                .header("origin", "http://testserver")
                .header("cookie", &cookie)
                .header("content-type", &content_type)
                .body(Body::from(multipart("clip.txt")))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/videos/subtitles/upload")
                .header("host", "testserver")
                .header("origin", "http://testserver")
                .header("cookie", &cookie)
                .header("content-type", &content_type)
                .body(Body::from(multipart("clip.mp4")))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let json = body_json(res).await;
    assert_eq!(json["segments"][0]["text"], "hello world");
    assert!(json["srt"].as_str().unwrap().contains("hello world"));

    let leftovers: Vec<_> = fs::read_dir(&settings.data_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(".upload-video-")
        })
        .collect();
    assert!(leftovers.is_empty(), "upload temp files must be removed");
}

#[tokio::test]
async fn translate_languages_and_text_prefer_english_russian() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("static")).unwrap();
    fs::write(root.join("static/index.html"), b"<html></html>").unwrap();
    let settings = test_settings(root);
    settings.create_data_dirs().unwrap();
    let db = Database::open(settings.database_path()).unwrap();
    db.set_admin(&video_work_api::security::hash_password("correct horse battery staple").unwrap())
        .unwrap();
    let studio = Arc::new(Studio::new(
        settings,
        db,
        Arc::new(FakeEngine::new()),
        Arc::new(FakeSubtitles::default()),
        Arc::new(FakeTranslationEngine::new()),
    ));
    let app = build_router(AppState {
        studio,
        limiter: Arc::new(LoginLimiter::new()),
        passkey_ceremonies: Arc::new(CeremonyStore::new()),
        model_downloads: Arc::new(ModelDownloadRegistry::new()),
    });

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .header("host", "testserver")
                .header("origin", "http://testserver")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"password":"correct horse battery staple"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let cookie = res
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/translate/languages")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let languages = body_json(res).await;
    assert_eq!(languages["languages"][0]["code"], "en");
    assert_eq!(languages["languages"][1]["code"], "ru");
    assert_eq!(languages["model"], "google/madlad400-3b-mt");

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/translate")
                .header("host", "testserver")
                .header("origin", "http://testserver")
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"target_lang":"ru","text":"hello"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let translated = body_json(res).await;
    assert_eq!(translated["target_lang"], "ru");
    assert_eq!(translated["text"], "[ru] hello");

    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/translate")
                .header("host", "testserver")
                .header("origin", "http://testserver")
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"target_lang":"en","srt":"1\n00:00:00,000 --> 00:00:01,000\n你好\n"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let srt_result = body_json(res).await;
    assert_eq!(srt_result["segments"][0]["text"], "[en] 你好");
    assert!(srt_result["srt"].as_str().unwrap().contains("[en] 你好"));
}
