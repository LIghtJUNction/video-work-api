use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Multipart, Path as AxumPath, Request, State};
use axum::http::{header, HeaderMap, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use serde::Deserialize;
use serde_json::{json, Value};
use tower_http::services::ServeDir;
use uuid::Uuid;

use super::limiter::LoginLimiter;
use crate::audio::{extension_allowed, MAX_UPLOAD_BYTES};
use crate::error::{AppError, AppResult};
use crate::mcp::{handle_mcp_message, McpResponse};
use crate::filenames::{content_disposition_attachment, download_name_from_text};
use crate::paths::safe_owned_file;
use crate::security::{
    constant_time_eq, hash_password, new_session_token, token_hash, verify_password,
};
use crate::studio::{Studio, StudioError};
use crate::{COOKIE_NAME, MAX_TEXT_LENGTH};

#[derive(Clone)]
pub struct AppState {
    pub studio: Arc<Studio>,
    pub limiter: Arc<LoginLimiter>,
}

pub fn build_router(state: AppState) -> Router {
    let static_dir = state.studio.settings.static_dir();
    let index = static_dir.join("index.html");
    let docs = static_dir.join("docs.html");

    let api = Router::new()
        .route("/api/status", get(status))
        .route("/api/setup", post(setup))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/speakers", get(list_speakers).post(add_speaker))
        .route("/api/speakers/{speaker_id}", delete(delete_speaker))
        .route(
            "/api/speakers/{speaker_id}/profiles",
            post(add_profile),
        )
        .route("/api/profiles/{profile_id}", delete(delete_profile))
        .route("/api/generations", post(generate))
        .route(
            "/api/generations/{generation_id}/audio",
            get(generation_audio),
        )
        .route("/api/videos/subtitles", post(video_subtitles))
        .route("/mcp", post(mcp_handler))
        .layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES as usize + 1024 * 1024));

    Router::new()
        .route("/", get(move || serve_html_file(index.clone())))
        .route("/docs", get(move || serve_html_file(docs.clone())))
        .nest_service("/static", ServeDir::new(static_dir))
        .merge(api)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            same_origin_middleware,
        ))
        .with_state(state)
}

async fn serve_html_file(path: PathBuf) -> impl IntoResponse {
    match fs::read(&path) {
        Ok(bytes) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            bytes,
        )
            .into_response(),
        Err(_) => AppError::not_found("page missing").into_response(),
    }
}

async fn same_origin_middleware(
    State(_state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    let path = req.uri().path().to_string();
    let method = req.method().clone();
    // MCP is the agent path: bearer auth only (no browser same-origin).
    if path == "/mcp" {
        return next.run(req).await;
    }
    if !matches!(method, Method::GET | Method::HEAD | Method::OPTIONS) {
        let origin = req.headers().get(header::ORIGIN).and_then(|v| v.to_str().ok());
        let fetch_site = req
            .headers()
            .get("sec-fetch-site")
            .and_then(|v| v.to_str().ok());
        if origin.is_none() || fetch_site == Some("cross-site") {
            return AppError::forbidden_origin().into_response();
        }
        let origin = origin.unwrap();
        let host = req
            .headers()
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if let Ok(url) = url::Url::parse(origin) {
            let scheme_ok = matches!(url.scheme(), "http" | "https");
            let origin_host = url.host_str().unwrap_or("");
            let origin_netloc = if let Some(port) = url.port() {
                format!("{origin_host}:{port}")
            } else {
                origin_host.to_string()
            };
            // When Host is missing (some test harnesses), compare hostnames only
            // after parsing; require host match when Host header is present.
            if !scheme_ok {
                return AppError::forbidden_origin().into_response();
            }
            if !host.is_empty() && !origin_netloc.eq_ignore_ascii_case(host) {
                return AppError::forbidden_origin().into_response();
            }
        } else {
            return AppError::forbidden_origin().into_response();
        }
    }
    next.run(req).await
}

fn mcp_bearer_ok(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(configured) = state.studio.settings.mcp_token.as_deref() else {
        return false;
    };
    let Some(auth) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let supplied = auth.strip_prefix("Bearer ").unwrap_or("").trim();
    !supplied.is_empty() && constant_time_eq(supplied, configured)
}

fn current_session(state: &AppState, jar: &CookieJar) -> bool {
    jar.get(COOKIE_NAME)
        .map(|c| c.value().to_string())
        .and_then(|token| {
            let digest = token_hash(&token);
            state.studio.database.session_exists(&digest).ok()
        })
        .unwrap_or(false)
}

fn require_auth(state: &AppState, jar: &CookieJar) -> AppResult<()> {
    if current_session(state, jar) {
        Ok(())
    } else {
        Err(AppError::unauthorized())
    }
}

async fn status(State(state): State<AppState>, jar: CookieJar) -> AppResult<Json<Value>> {
    let auth = current_session(&state, &jar);
    Ok(Json(state.studio.status_payload(auth)?))
}

#[derive(Deserialize)]
struct SetupBody {
    token: String,
    password: String,
}

async fn setup(State(state): State<AppState>, Json(body): Json<SetupBody>) -> AppResult<Response> {
    if state.studio.database.configured()? {
        return Err(AppError::api(
            StatusCode::CONFLICT,
            "already_configured",
            "Setup is already complete",
        ));
    }
    if body.password.len() < 12 {
        return Err(AppError::invalid_request(
            "Password must contain at least 12 characters",
        ));
    }
    let (expected, identity) = read_setup_token(&state.studio.settings.setup_token_file)
        .map_err(|_| {
            AppError::api(
                StatusCode::CONFLICT,
                "setup_unavailable",
                "Setup token is unavailable",
            )
        })?;
    if !constant_time_eq(&body.token, &expected) {
        return Err(AppError::api(
            StatusCode::FORBIDDEN,
            "invalid_setup_token",
            "Setup token is invalid",
        ));
    }
    let digest = hash_password(&body.password)
        .map_err(|e| AppError::Internal(e))?;
    if !state.studio.database.set_admin(&digest)? {
        return Err(AppError::api(
            StatusCode::CONFLICT,
            "already_configured",
            "Setup is already complete",
        ));
    }
    if let Err(e) = invalidate_setup_token(&state.studio.settings.setup_token_file, identity) {
        let _ = state.studio.database.delete_admin_hash(&digest);
        tracing::error!(error = %e, "Setup token invalidation failed");
        return Err(AppError::api(
            StatusCode::INTERNAL_SERVER_ERROR,
            "setup_incomplete",
            "Setup token could not be invalidated",
        ));
    }
    Ok((StatusCode::CREATED, Json(json!({ "configured": true }))).into_response())
}

#[derive(Deserialize)]
struct LoginBody {
    password: String,
}

async fn login(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(body): Json<LoginBody>,
) -> AppResult<Response> {
    let key = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown");
    if !state.limiter.allow(key) {
        return Err(AppError::api(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "Too many login attempts",
        ));
    }
    let admin = state.studio.database.admin_password_hash()?;
    let Some(hash) = admin else {
        return Err(AppError::api(
            StatusCode::UNAUTHORIZED,
            "invalid_credentials",
            "Invalid password",
        ));
    };
    if !verify_password(&body.password, &hash) {
        return Err(AppError::api(
            StatusCode::UNAUTHORIZED,
            "invalid_credentials",
            "Invalid password",
        ));
    }
    let token = new_session_token();
    state.studio.database.create_session(&token_hash(&token))?;
    let secure = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        == Some("https");
    let cookie = Cookie::build((COOKIE_NAME, token))
        .http_only(true)
        .same_site(SameSite::Strict)
        .path("/")
        .secure(secure)
        .build();
    let jar = jar.add(cookie);
    Ok((jar, Json(json!({ "authenticated": true }))).into_response())
}

async fn logout(State(state): State<AppState>, jar: CookieJar) -> AppResult<Response> {
    require_auth(&state, &jar)?;
    if let Some(c) = jar.get(COOKIE_NAME) {
        let _ = state
            .studio
            .database
            .delete_session(&token_hash(c.value()));
    }
    let jar = jar.remove(Cookie::from(COOKIE_NAME));
    Ok((jar, Json(json!({ "authenticated": false }))).into_response())
}

async fn list_speakers(State(state): State<AppState>, jar: CookieJar) -> AppResult<Json<Value>> {
    require_auth(&state, &jar)?;
    Ok(Json(state.studio.list_speakers()?))
}

#[derive(Deserialize)]
struct SpeakerBody {
    name: String,
}

async fn add_speaker(
    State(state): State<AppState>,
    jar: CookieJar,
    Json(body): Json<SpeakerBody>,
) -> AppResult<Response> {
    require_auth(&state, &jar)?;
    let result = state.studio.create_speaker(&body.name).map_err(|e| {
        AppError::invalid_request(e.to_string())
    })?;
    Ok((StatusCode::CREATED, Json(result)).into_response())
}

async fn delete_speaker(
    State(state): State<AppState>,
    jar: CookieJar,
    AxumPath(speaker_id): AxumPath<String>,
) -> AppResult<StatusCode> {
    require_auth(&state, &jar)?;
    match state.studio.delete_speaker(&speaker_id) {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(e) => {
            if let Some(StudioError::SpeakerHasProfiles) = e.downcast_ref() {
                Err(AppError::api(
                    StatusCode::CONFLICT,
                    "speaker_has_profiles",
                    "Delete profiles first",
                ))
            } else if let Some(StudioError::SpeakerNotFound) = e.downcast_ref() {
                Err(AppError::not_found("Speaker not found"))
            } else {
                Err(AppError::Internal(e))
            }
        }
    }
}

async fn add_profile(
    State(state): State<AppState>,
    jar: CookieJar,
    AxumPath(speaker_id): AxumPath<String>,
    mut multipart: Multipart,
) -> AppResult<Response> {
    require_auth(&state, &jar)?;
    if state
        .studio
        .database
        .speaker_by_id(&speaker_id)?
        .is_none()
    {
        return Err(AppError::not_found("Speaker not found"));
    }

    let mut style_name = String::new();
    let mut prompt_text = String::new();
    let mut consent = false;
    let mut upload_path: Option<PathBuf> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::invalid_request(e.to_string()))?
    {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "style_name" => {
                style_name = field
                    .text()
                    .await
                    .map_err(|e| AppError::invalid_request(e.to_string()))?
                    .trim()
                    .to_string();
            }
            "prompt_text" => {
                prompt_text = field
                    .text()
                    .await
                    .map_err(|e| AppError::invalid_request(e.to_string()))?
                    .trim()
                    .to_string();
            }
            "consent" => {
                let v = field
                    .text()
                    .await
                    .map_err(|e| AppError::invalid_request(e.to_string()))?;
                consent = matches!(v.trim(), "true" | "1" | "on" | "yes");
            }
            "audio" => {
                let original_name = field.file_name().unwrap_or("audio.wav").to_string();
                let suffix = Path::new(&original_name)
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| format!(".{}", e.to_ascii_lowercase()))
                    .unwrap_or_else(|| ".wav".into());
                if !extension_allowed(Path::new(&format!("x{suffix}"))) {
                    return Err(AppError::api(
                        StatusCode::UNSUPPORTED_MEDIA_TYPE,
                        "unsupported_audio",
                        "Unsupported audio format",
                    ));
                }
                let temp = state
                    .studio
                    .settings
                    .data_dir
                    .join(format!(".upload-{}{}", Uuid::new_v4(), suffix));
                let mut file = File::create(&temp).map_err(|e| AppError::Internal(e.into()))?;
                let data = field
                    .bytes()
                    .await
                    .map_err(|e| AppError::invalid_request(e.to_string()))?;
                if data.len() as u64 > MAX_UPLOAD_BYTES {
                    let _ = fs::remove_file(&temp);
                    return Err(AppError::api(
                        StatusCode::PAYLOAD_TOO_LARGE,
                        "upload_too_large",
                        "Audio exceeds 50 MiB",
                    ));
                }
                file.write_all(&data)
                    .map_err(|e| AppError::Internal(e.into()))?;
                upload_path = Some(temp);
            }
            _ => {}
        }
    }

    if !consent {
        if let Some(p) = &upload_path {
            let _ = fs::remove_file(p);
        }
        return Err(AppError::api(
            StatusCode::UNPROCESSABLE_ENTITY,
            "rights_required",
            "Confirm that you have rights to this voice",
        ));
    }
    if style_name.is_empty()
        || style_name.len() > 100
        || prompt_text.is_empty()
        || prompt_text.len() > 2000
    {
        if let Some(p) = &upload_path {
            let _ = fs::remove_file(p);
        }
        return Err(AppError::invalid_request(
            "Style and exact transcript are required",
        ));
    }
    let Some(upload_path) = upload_path else {
        return Err(AppError::invalid_request("Audio file is required"));
    };

    let studio = state.studio.clone();
    let speaker_id_c = speaker_id.clone();
    let style_c = style_name.clone();
    let prompt_c = prompt_text.clone();
    let upload_c = upload_path.clone();
    let result = tokio::task::spawn_blocking(move || {
        studio.add_profile_from_file(&speaker_id_c, &style_c, &prompt_c, &upload_c, true)
    })
    .await
    .map_err(|e| AppError::Internal(e.into()))?;

    let _ = fs::remove_file(&upload_path);

    match result {
        Ok(v) => Ok((StatusCode::CREATED, Json(v)).into_response()),
        Err(e) => {
            if let Some(StudioError::RightsRequired) = e.downcast_ref() {
                Err(AppError::api(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "rights_required",
                    "Confirm that you have rights to this voice",
                ))
            } else if let Some(StudioError::UnsupportedAudio) = e.downcast_ref() {
                Err(AppError::api(
                    StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    "unsupported_audio",
                    "Unsupported audio format",
                ))
            } else if let Some(StudioError::InvalidAudio) = e.downcast_ref() {
                Err(AppError::api(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "invalid_audio",
                    "Audio conversion or validation failed",
                ))
            } else if let Some(StudioError::ProfileFailed) = e.downcast_ref() {
                Err(AppError::api(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "profile_failed",
                    "Profile could not be saved",
                ))
            } else {
                Err(AppError::api(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "invalid_audio",
                    "Audio conversion or validation failed",
                ))
            }
        }
    }
}

async fn delete_profile(
    State(state): State<AppState>,
    jar: CookieJar,
    AxumPath(profile_id): AxumPath<String>,
) -> AppResult<StatusCode> {
    require_auth(&state, &jar)?;
    match state.studio.delete_profile(&profile_id) {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(e) => {
            if let Some(StudioError::ProfileNotFound) = e.downcast_ref() {
                Err(AppError::not_found("Profile not found"))
            } else if let Some(StudioError::ProfileInUse) = e.downcast_ref() {
                Err(AppError::api(
                    StatusCode::CONFLICT,
                    "profile_in_use",
                    "Profile has generation history",
                ))
            } else if let Some(StudioError::ProfileFileInvalid) = e.downcast_ref() {
                Err(AppError::api(
                    StatusCode::CONFLICT,
                    "profile_file_invalid",
                    "Profile audio is invalid",
                ))
            } else {
                Err(AppError::Internal(e))
            }
        }
    }
}

#[derive(Deserialize)]
struct GenerationBody {
    speaker_id: String,
    profile_id: String,
    target_text: String,
    #[serde(default = "default_speed")]
    speed: f64,
}

fn default_speed() -> f64 {
    1.0
}

async fn generate(
    State(state): State<AppState>,
    jar: CookieJar,
    Json(body): Json<GenerationBody>,
) -> AppResult<Response> {
    require_auth(&state, &jar)?;
    let text = body.target_text.trim();
    if text.is_empty() || text.len() > MAX_TEXT_LENGTH {
        return Err(AppError::invalid_request(
            "Text must contain 1 to 1200 characters",
        ));
    }
    if !(0.75..=1.25).contains(&body.speed) || !body.speed.is_finite() {
        return Err(AppError::invalid_request(
            "Speed must be between 0.75 and 1.25",
        ));
    }
    let studio = state.studio.clone();
    let speaker_id = body.speaker_id.clone();
    let profile_id = body.profile_id.clone();
    let target = text.to_string();
    let speed = body.speed;
    let result = tokio::task::spawn_blocking(move || {
        studio.generate_speech(&speaker_id, &profile_id, &target, speed)
    })
    .await
    .map_err(|e| AppError::Internal(e.into()))?;

    match result {
        Ok(v) => {
            let slim = json!({
                "id": v.get("id"),
                "audio_url": v.get("audio_url"),
                "audio": v.get("audio"),
                // Browser <a download> prefers a human-readable basename.
                "download_name": v
                    .get("download_name")
                    .cloned()
                    .unwrap_or_else(|| json!(download_name_from_text(text))),
            });
            Ok((StatusCode::CREATED, Json(slim)).into_response())
        }
        Err(e) => {
            if let Some(StudioError::InvalidProfile) = e.downcast_ref() {
                Err(AppError::api(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "invalid_profile",
                    "Profile does not belong to speaker",
                ))
            } else if let Some(StudioError::ProfileFileInvalid) = e.downcast_ref() {
                Err(AppError::api(
                    StatusCode::CONFLICT,
                    "profile_file_invalid",
                    "Profile audio is unavailable",
                ))
            } else {
                Err(AppError::api(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "generation_failed",
                    "Audio generation failed; check service logs",
                ))
            }
        }
    }
}

async fn generation_audio(
    State(state): State<AppState>,
    jar: CookieJar,
    AxumPath(generation_id): AxumPath<String>,
) -> AppResult<Response> {
    require_auth(&state, &jar)?;
    let row = state
        .studio
        .database
        .generation_by_id(&generation_id)?
        .filter(|r| r.status == "complete");
    let Some(row) = row else {
        return Err(AppError::not_found("Generated audio not found"));
    };
    let path = row
        .audio_name
        .as_ref()
        .and_then(|n| safe_owned_file(&state.studio.settings.generations_dir(), n));
    let Some(path) = path else {
        return Err(AppError::not_found("Generated audio not found"));
    };
    let mut file = File::open(&path).map_err(|e| AppError::Internal(e.into()))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)
        .map_err(|e| AppError::Internal(e.into()))?;
    // Prefer 前缀…后缀.wav over opaque generation UUID.
    let disposition = content_disposition_attachment(&row.target_text);
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "audio/wav")
        .header(header::CONTENT_DISPOSITION, disposition)
        .body(Body::from(buf))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()))
}

#[derive(Deserialize)]
struct SubtitleBody {
    video_path: String,
}

async fn video_subtitles(
    State(state): State<AppState>,
    jar: CookieJar,
    Json(body): Json<SubtitleBody>,
) -> AppResult<Json<Value>> {
    require_auth(&state, &jar)?;
    let video_path = body.video_path.trim().to_string();
    if video_path.is_empty() || video_path.len() > 4096 {
        return Err(AppError::invalid_request("video_path is required"));
    }
    let studio = state.studio.clone();
    let result = tokio::task::spawn_blocking(move || studio.extract_subtitles(&video_path))
        .await
        .map_err(|e| AppError::Internal(e.into()))?;
    match result {
        Ok(v) => Ok(Json(v)),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("must be inside") {
                Err(AppError::api(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "invalid_video_path",
                    msg,
                ))
            } else {
                tracing::warn!(error = %msg, "Subtitle extraction failed");
                Err(AppError::api(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "subtitle_extraction_failed",
                    msg,
                ))
            }
        }
    }
}

async fn mcp_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<Value>, axum::extract::rejection::JsonRejection>,
) -> Response {
    if !mcp_bearer_ok(&state, &headers) {
        return AppError::api(
            StatusCode::UNAUTHORIZED,
            "authentication_required",
            "Use Authorization: Bearer <VWA_MCP_TOKEN>",
        )
        .into_response();
    }
    let Json(message) = match body {
        Ok(j) => j,
        Err(_) => {
            return AppError::api(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "MCP request must be JSON",
            )
            .into_response();
        }
    };
    if !message.is_object() {
        return AppError::api(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "MCP request must be a JSON object",
        )
        .into_response();
    }
    // Tools that may block (generation / subtitles) run on blocking pool when needed
    // inside studio; MCP handler itself is sync for simplicity via spawn_blocking.
    let studio = state.studio.clone();
    match tokio::task::spawn_blocking(move || handle_mcp_message(&studio, message)).await {
        Ok(McpResponse::Json(v)) => Json(v).into_response(),
        Ok(McpResponse::Accepted) => StatusCode::ACCEPTED.into_response(),
        Err(e) => AppError::Internal(e.into()).into_response(),
    }
}

fn read_setup_token(path: &Path) -> anyhow::Result<(String, (u64, u64))> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        anyhow::bail!("Setup token is not a regular file");
    }
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        (meta.dev(), meta.ino())
    };
    #[cfg(not(unix))]
    let identity = (0u64, 0u64);

    let token = fs::read_to_string(path)?.trim().to_string();
    if token.is_empty() {
        anyhow::bail!("Setup token is empty");
    }
    Ok((token, identity))
}

fn invalidate_setup_token(path: &Path, identity: (u64, u64)) -> anyhow::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        anyhow::bail!("Setup token changed");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if (meta.dev(), meta.ino()) != identity {
            anyhow::bail!("Setup token changed");
        }
    }
    fs::remove_file(path)?;
    Ok(())
}

// Silence unused import if Body unused in some builds.
#[allow(dead_code)]
fn _body_type(_: Body) {}
