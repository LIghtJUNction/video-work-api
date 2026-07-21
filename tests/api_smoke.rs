//! HTTP smoke tests with a fake speech engine.

use std::fs;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tempfile::tempdir;
use tower::ServiceExt;
use video_work_api::config::Settings;
use video_work_api::database::Database;
use video_work_api::engine::FakeEngine;
use video_work_api::http::{build_router, AppState, LoginLimiter};
use video_work_api::model::ModelDownloadManager;
use video_work_api::passkeys::CeremonyStore;
use video_work_api::studio::Studio;
use video_work_api::subtitles::FakeSubtitles;

fn test_settings(root: &std::path::Path) -> Settings {
    Settings {
        data_dir: root.to_path_buf(),
        model_dir: root.join("model"),
        cosyvoice_root: root.join("source"),
        setup_token_file: root.join("setup-token"),
        host: "127.0.0.1".into(),
        port: 7860,
        ssl_certfile: None,
        ssl_keyfile: None,
        mcp_token: Some("mcp-secret-token".into()),
        funclip_root: None,
        video_input_dir: root.join("videos"),
        reference_input_dir: root.join("references"),
        subtitle_timeout_seconds: 30,
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
    ));
    let app = build_router(AppState {
        studio,
        limiter: Arc::new(LoginLimiter::new()),
        passkey_ceremonies: Arc::new(CeremonyStore::new()),
        model_download: Arc::new(ModelDownloadManager::new()),
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
    ));
    let app = build_router(AppState {
        studio,
        limiter: Arc::new(LoginLimiter::new()),
        passkey_ceremonies: Arc::new(CeremonyStore::new()),
        model_download: Arc::new(ModelDownloadManager::new()),
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
    ));
    let app = build_router(AppState {
        studio,
        limiter: Arc::new(LoginLimiter::new()),
        passkey_ceremonies: Arc::new(CeremonyStore::new()),
        model_download: Arc::new(ModelDownloadManager::new()),
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
    assert!(json["result"]["tools"].as_array().unwrap().len() >= 5);
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
    ));
    let app = build_router(AppState {
        studio,
        limiter: Arc::new(LoginLimiter::new()),
        passkey_ceremonies: Arc::new(CeremonyStore::new()),
        model_download: Arc::new(ModelDownloadManager::new()),
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
    ));
    let app = build_router(AppState {
        studio,
        limiter: Arc::new(LoginLimiter::new()),
        passkey_ceremonies: Arc::new(CeremonyStore::new()),
        model_download: Arc::new(ModelDownloadManager::new()),
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
