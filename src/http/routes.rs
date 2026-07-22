use std::fs::{self, File};
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{ConnectInfo, DefaultBodyLimit, Multipart, Path as AxumPath, Request, State};
use axum::http::{header, HeaderMap, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, patch, post};
use axum::{Extension, Json, Router};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::Deserialize;
use serde_json::{json, Value};
use tower_http::services::ServeDir;
use uuid::Uuid;
use webauthn_rs::prelude::{Passkey, PublicKeyCredential, RegisterPublicKeyCredential};

use super::limiter::LoginLimiter;
use crate::audio::{extension_allowed, MAX_UPLOAD_BYTES};
use crate::error::{AppError, AppResult};
use crate::filenames::{content_disposition_attachment, download_name_from_text};
use crate::mcp::{handle_mcp_message, McpResponse};
use crate::model::{download_model, model_files_present, ModelDownloadManager};
use crate::passkeys::{CeremonyStore, CeremonyStoreError, PasskeyWebauthn, PendingCeremony};
use crate::paths::safe_owned_file;
use crate::security::{
    constant_time_eq, hash_password, is_admin_password_valid, new_session_token, token_hash,
    verify_password,
};
use crate::studio::{Studio, StudioError};
use crate::subtitles::{video_extension_allowed, MAX_VIDEO_UPLOAD_BYTES};
use crate::{target_text_is_valid, COOKIE_NAME};

#[derive(Clone)]
pub struct AppState {
    pub studio: Arc<Studio>,
    pub limiter: Arc<LoginLimiter>,
    pub passkey_ceremonies: Arc<CeremonyStore>,
    pub model_download: Arc<ModelDownloadManager>,
}

pub fn build_router(state: AppState) -> Router {
    let _ = state.passkey_ceremonies.cleanup();
    let static_dir = state.studio.settings.static_dir();
    let index = static_dir.join("index.html");
    let docs = static_dir.join("docs.html");

    let api = Router::new()
        .route("/api/status", get(status))
        .route("/api/setup", post(setup))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/mcp-token", post(get_mcp_token))
        .route(
            "/api/model/download",
            get(model_download_status).post(start_model_download),
        )
        .route("/api/auth/passkeys", get(list_passkeys))
        .route(
            "/api/auth/passkeys/register/start",
            post(start_passkey_registration),
        )
        .route(
            "/api/auth/passkeys/register/finish",
            post(finish_passkey_registration),
        )
        .route(
            "/api/auth/passkeys/{passkey_id}",
            axum::routing::delete(delete_passkey),
        )
        .route("/api/auth/passkeys/login/start", post(start_passkey_login))
        .route(
            "/api/auth/passkeys/login/finish",
            post(finish_passkey_login),
        )
        .route("/api/speakers", get(list_speakers).post(add_speaker))
        .route(
            "/api/speakers/{speaker_id}",
            patch(rename_speaker).delete(delete_speaker),
        )
        .route("/api/speakers/{speaker_id}/profiles", post(add_profile))
        .route(
            "/api/profiles/{profile_id}",
            patch(rename_profile).delete(delete_profile),
        )
        .route("/api/generations", post(generate))
        .route(
            "/api/generations/{generation_id}/audio",
            get(generation_audio),
        )
        .route("/api/videos/subtitles", post(video_subtitles))
        .route(
            "/api/videos/subtitles/upload",
            post(video_subtitles_upload).layer(DefaultBodyLimit::max(
                MAX_VIDEO_UPLOAD_BYTES as usize + 1024 * 1024,
            )),
        )
        .route("/mcp", post(mcp_handler))
        .layer(DefaultBodyLimit::max(
            MAX_UPLOAD_BYTES as usize + 1024 * 1024,
        ));

    Router::new()
        .route("/", get(move || serve_html_file(index.clone())))
        .route("/docs", get(move || serve_html_file(docs.clone())))
        .route(
            "/favicon.ico",
            get(|| async { Redirect::temporary("/static/favicon.svg") }),
        )
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
        let origin = req
            .headers()
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok());
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
            let origin_host = match url.host() {
                Some(url::Host::Domain(domain)) => domain.to_string(),
                Some(url::Host::Ipv4(ip)) => ip.to_string(),
                Some(url::Host::Ipv6(ip)) => format!("[{ip}]"),
                None => String::new(),
            };
            let origin_netloc = url
                .port()
                .map_or(origin_host.clone(), |port| format!("{origin_host}:{port}"));
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
    let Some(auth) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    else {
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

fn login_rate_key(peer: Option<&Extension<ConnectInfo<SocketAddr>>>) -> String {
    peer.map(|Extension(ConnectInfo(address))| address.ip().to_string())
        .unwrap_or_else(|| "unknown-peer".to_string())
}

fn enforce_login_rate_limit(
    state: &AppState,
    peer: Option<&Extension<ConnectInfo<SocketAddr>>>,
) -> AppResult<()> {
    if state.limiter.allow(&login_rate_key(peer)) {
        Ok(())
    } else {
        Err(AppError::api(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "Too many login attempts",
        ))
    }
}

fn authenticated_response(
    state: &AppState,
    jar: CookieJar,
    headers: &HeaderMap,
) -> AppResult<Response> {
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
    Ok((jar.add(cookie), Json(json!({ "authenticated": true }))).into_response())
}

fn webauthn_from_origin(headers: &HeaderMap) -> AppResult<PasskeyWebauthn> {
    let raw_origin = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            AppError::api(
                StatusCode::BAD_REQUEST,
                "invalid_webauthn_origin",
                "Passkeys require a valid browser origin",
            )
        })?;
    let origin = url::Url::parse(raw_origin).map_err(|_| {
        AppError::api(
            StatusCode::BAD_REQUEST,
            "invalid_webauthn_origin",
            "Passkeys require a valid browser origin",
        )
    })?;
    let host = match origin.host() {
        Some(url::Host::Domain(domain)) => domain.to_string(),
        Some(url::Host::Ipv4(_)) | Some(url::Host::Ipv6(_)) => {
            return Err(AppError::api(
                StatusCode::BAD_REQUEST,
                "webauthn_ip_origin_unsupported",
                "Passkeys do not support IP address origins; use http://localhost:<port> or an HTTPS domain",
            ));
        }
        None => {
            return Err(AppError::api(
                StatusCode::BAD_REQUEST,
                "invalid_webauthn_origin",
                "Passkeys require an origin with a hostname",
            ));
        }
    };
    let localhost = host.eq_ignore_ascii_case("localhost");
    if origin.scheme() != "https" && !(origin.scheme() == "http" && localhost) {
        return Err(AppError::api(
            StatusCode::BAD_REQUEST,
            "webauthn_https_required",
            "Passkeys require an HTTPS domain; only localhost may use HTTP for development",
        ));
    }
    if origin.username() != ""
        || origin.password().is_some()
        || origin.query().is_some()
        || origin.fragment().is_some()
        || origin.path() != "/"
    {
        return Err(AppError::api(
            StatusCode::BAD_REQUEST,
            "invalid_webauthn_origin",
            "Passkeys require a plain origin without a path, query, or credentials",
        ));
    }
    PasskeyWebauthn::new(&host, origin).map_err(|_| {
        AppError::api(
            StatusCode::BAD_REQUEST,
            "invalid_webauthn_origin",
            "Passkeys require a valid browser origin",
        )
    })
}

fn insert_passkey_ceremony(state: &AppState, ceremony: PendingCeremony) -> AppResult<String> {
    match state.passkey_ceremonies.insert(ceremony) {
        Ok(transaction_id) => Ok(transaction_id),
        Err(CeremonyStoreError::Full) => Err(AppError::api(
            StatusCode::SERVICE_UNAVAILABLE,
            "passkey_ceremony_capacity",
            "Too many passkey requests are pending; try again shortly",
        )),
        Err(error) => Err(AppError::Internal(error.into())),
    }
}

fn load_stored_passkeys(state: &AppState) -> AppResult<Vec<(String, Passkey)>> {
    state
        .studio
        .database
        .load_passkeys()?
        .into_iter()
        .map(|row| {
            serde_json::from_str::<Passkey>(&row.credential_json)
                .map(|passkey| (row.id, passkey))
                .map_err(|error| AppError::Internal(error.into()))
        })
        .collect()
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
    if !is_admin_password_valid(&body.password) {
        return Err(AppError::invalid_request(
            "Password must contain at least 12 characters",
        ));
    }
    let (expected, identity) =
        read_setup_token(&state.studio.settings.setup_token_file).map_err(|_| {
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
    let digest = hash_password(&body.password).map_err(AppError::Internal)?;
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
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    Json(body): Json<LoginBody>,
) -> AppResult<Response> {
    enforce_login_rate_limit(&state, peer.as_ref())?;
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
    authenticated_response(&state, jar, &headers)
}

async fn list_passkeys(State(state): State<AppState>, jar: CookieJar) -> AppResult<Json<Value>> {
    require_auth(&state, &jar)?;
    Ok(Json(
        json!({ "passkeys": state.studio.database.list_passkeys()? }),
    ))
}

#[derive(Deserialize)]
struct RegisterPasskeyStartBody {
    name: String,
}

async fn start_passkey_registration(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(body): Json<RegisterPasskeyStartBody>,
) -> AppResult<Json<Value>> {
    require_auth(&state, &jar)?;
    let name = body.name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(AppError::invalid_request(
            "Passkey name must contain 1 to 100 characters",
        ));
    }
    let webauthn = webauthn_from_origin(&headers)?;
    let passkeys = load_stored_passkeys(&state)?;
    let excludes = (!passkeys.is_empty()).then(|| {
        passkeys
            .iter()
            .map(|(_, key)| key.cred_id().clone())
            .collect()
    });
    let (challenge, registration) = webauthn
        .start_passkey_registration(
            Uuid::from_u128(0x4f7b_7a95_a5f4_4b47_a473_11dc_f2c5_c106),
            "admin",
            "Administrator",
            excludes,
        )
        .map_err(|_| {
            AppError::api(
                StatusCode::BAD_REQUEST,
                "passkey_registration_failed",
                "Could not start passkey registration",
            )
        })?;
    let transaction_id = insert_passkey_ceremony(
        &state,
        PendingCeremony::registration(name.to_string(), webauthn, registration),
    )?;
    Ok(Json(json!({
        "transaction_id": transaction_id,
        "publicKey": challenge.public_key,
    })))
}

#[derive(Deserialize)]
struct RegisterPasskeyFinishBody {
    transaction_id: String,
    credential: RegisterPublicKeyCredential,
}

async fn finish_passkey_registration(
    State(state): State<AppState>,
    jar: CookieJar,
    Json(body): Json<RegisterPasskeyFinishBody>,
) -> AppResult<Response> {
    require_auth(&state, &jar)?;
    let pending = state
        .passkey_ceremonies
        .take(&body.transaction_id)
        .map_err(|error| AppError::Internal(error.into()))?;
    let Some(PendingCeremony::Registration {
        name,
        webauthn,
        state: registration,
        ..
    }) = pending
    else {
        return Err(AppError::api(
            StatusCode::BAD_REQUEST,
            "invalid_passkey_transaction",
            "Passkey registration expired or was already used",
        ));
    };
    let passkey = webauthn
        .finish_passkey_registration(&body.credential, &registration)
        .map_err(|_| {
            AppError::api(
                StatusCode::BAD_REQUEST,
                "passkey_verification_failed",
                "Passkey registration could not be verified",
            )
        })?;
    let credential_id = URL_SAFE_NO_PAD.encode(passkey.cred_id().as_ref());
    if state
        .studio
        .database
        .load_passkeys()?
        .iter()
        .any(|row| row.credential_id == credential_id)
    {
        return Err(AppError::api(
            StatusCode::CONFLICT,
            "passkey_already_registered",
            "This passkey is already registered",
        ));
    }
    let credential_json =
        serde_json::to_string(&passkey).map_err(|error| AppError::Internal(error.into()))?;
    let row = state.studio.database.insert_passkey(
        &Uuid::new_v4().to_string(),
        &name,
        &credential_id,
        &credential_json,
    )?;
    let Some(row) = row else {
        return Err(AppError::api(
            StatusCode::CONFLICT,
            "passkey_already_registered",
            "This passkey is already registered",
        ));
    };
    Ok((StatusCode::CREATED, Json(row)).into_response())
}

async fn delete_passkey(
    State(state): State<AppState>,
    jar: CookieJar,
    AxumPath(passkey_id): AxumPath<String>,
) -> AppResult<Response> {
    require_auth(&state, &jar)?;
    if state.studio.database.delete_passkey(&passkey_id)? {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Err(AppError::not_found("Passkey not found"))
    }
}

async fn start_passkey_login(
    State(state): State<AppState>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    enforce_login_rate_limit(&state, peer.as_ref())?;
    let webauthn = webauthn_from_origin(&headers)?;
    let stored = load_stored_passkeys(&state)?;
    if stored.is_empty() {
        return Err(AppError::api(
            StatusCode::CONFLICT,
            "no_passkeys",
            "No passkeys are registered; sign in with the password first",
        ));
    }
    let passkeys: Vec<Passkey> = stored.into_iter().map(|(_, key)| key).collect();
    let (challenge, authentication) =
        webauthn
            .start_passkey_authentication(&passkeys)
            .map_err(|_| {
                AppError::api(
                    StatusCode::BAD_REQUEST,
                    "passkey_login_failed",
                    "Could not start passkey login",
                )
            })?;
    let transaction_id = insert_passkey_ceremony(
        &state,
        PendingCeremony::authentication(webauthn, authentication),
    )?;
    Ok(Json(json!({
        "transaction_id": transaction_id,
        "publicKey": challenge.public_key,
    })))
}

#[derive(Deserialize)]
struct PasskeyLoginFinishBody {
    transaction_id: String,
    credential: PublicKeyCredential,
}

async fn finish_passkey_login(
    State(state): State<AppState>,
    jar: CookieJar,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    Json(body): Json<PasskeyLoginFinishBody>,
) -> AppResult<Response> {
    enforce_login_rate_limit(&state, peer.as_ref())?;
    let pending = state
        .passkey_ceremonies
        .take(&body.transaction_id)
        .map_err(|error| AppError::Internal(error.into()))?;
    let Some(PendingCeremony::Authentication {
        webauthn,
        state: authentication,
        ..
    }) = pending
    else {
        return Err(AppError::api(
            StatusCode::BAD_REQUEST,
            "invalid_passkey_transaction",
            "Passkey login expired or was already used",
        ));
    };
    let result = webauthn
        .finish_passkey_authentication(&body.credential, &authentication)
        .map_err(|_| {
            AppError::api(
                StatusCode::UNAUTHORIZED,
                "passkey_verification_failed",
                "Passkey login could not be verified",
            )
        })?;
    let credential_id = URL_SAFE_NO_PAD.encode(result.cred_id().as_ref());
    let stored = state.studio.database.load_passkeys()?;
    let Some(row) = stored
        .into_iter()
        .find(|row| row.credential_id == credential_id)
    else {
        return Err(AppError::api(
            StatusCode::UNAUTHORIZED,
            "passkey_verification_failed",
            "Passkey login could not be verified",
        ));
    };
    let mut passkey: Passkey = serde_json::from_str(&row.credential_json)
        .map_err(|error| AppError::Internal(error.into()))?;
    if passkey.update_credential(&result) == Some(true) {
        let credential_json =
            serde_json::to_string(&passkey).map_err(|error| AppError::Internal(error.into()))?;
        state
            .studio
            .database
            .update_passkey(&row.id, &credential_json)?;
    }
    authenticated_response(&state, jar, &headers)
}

async fn logout(State(state): State<AppState>, jar: CookieJar) -> AppResult<Response> {
    require_auth(&state, &jar)?;
    if let Some(c) = jar.get(COOKIE_NAME) {
        let _ = state.studio.database.delete_session(&token_hash(c.value()));
    }
    let jar = jar.remove(Cookie::from(COOKIE_NAME));
    Ok((jar, Json(json!({ "authenticated": false }))).into_response())
}

async fn get_mcp_token(State(state): State<AppState>, jar: CookieJar) -> AppResult<Response> {
    require_auth(&state, &jar)?;
    let token = state.studio.settings.mcp_token.as_deref().ok_or_else(|| {
        AppError::api(
            StatusCode::NOT_FOUND,
            "mcp_not_configured",
            "The MCP token is not configured",
        )
    })?;
    Ok((
        [
            (
                header::CACHE_CONTROL,
                "no-store, max-age=0, must-revalidate",
            ),
            (header::PRAGMA, "no-cache"),
            (header::EXPIRES, "0"),
        ],
        Json(json!({ "token": token })),
    )
        .into_response())
}

async fn model_download_status(
    State(state): State<AppState>,
    jar: CookieJar,
) -> AppResult<Json<Value>> {
    require_auth(&state, &jar)?;
    Ok(Json(json!({
        "state": state.model_download.state().as_str(),
        "model_present": model_files_present(&state.studio.settings),
    })))
}

async fn start_model_download(
    State(state): State<AppState>,
    jar: CookieJar,
) -> AppResult<Response> {
    require_auth(&state, &jar)?;
    if model_files_present(&state.studio.settings) {
        return Ok((
            StatusCode::OK,
            Json(json!({ "state": "succeeded", "model_present": true })),
        )
            .into_response());
    }
    if !state.model_download.try_start() {
        return Err(AppError::api(
            StatusCode::CONFLICT,
            "model_download_running",
            "Model download is already running",
        ));
    }
    let manager = state.model_download.clone();
    let settings = state.studio.settings.clone();
    tokio::spawn(async move {
        let result = tokio::task::spawn_blocking(move || download_model(&settings)).await;
        match result {
            Ok(Ok(())) => manager.finish(true),
            Ok(Err(error)) => {
                tracing::error!(error = %error, "Model download failed");
                manager.finish(false);
            }
            Err(error) => {
                tracing::error!(error = %error, "Model download task failed");
                manager.finish(false);
            }
        }
    });
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "state": "running", "model_present": false })),
    )
        .into_response())
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
    let result = state
        .studio
        .create_speaker(&body.name)
        .map_err(|e| AppError::invalid_request(e.to_string()))?;
    Ok((StatusCode::CREATED, Json(result)).into_response())
}

async fn rename_speaker(
    State(state): State<AppState>,
    jar: CookieJar,
    AxumPath(speaker_id): AxumPath<String>,
    Json(body): Json<SpeakerBody>,
) -> AppResult<Response> {
    require_auth(&state, &jar)?;
    match state.studio.rename_speaker(&speaker_id, &body.name) {
        Ok(v) => Ok((StatusCode::OK, Json(v)).into_response()),
        Err(e) => {
            if let Some(StudioError::SpeakerNotFound) = e.downcast_ref() {
                Err(AppError::not_found("Speaker not found"))
            } else if let Some(StudioError::NameConflict) = e.downcast_ref() {
                Err(AppError::api(
                    StatusCode::CONFLICT,
                    "name_conflict",
                    "A speaker with this name already exists",
                ))
            } else {
                Err(AppError::invalid_request(e.to_string()))
            }
        }
    }
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
    if state.studio.database.speaker_by_id(&speaker_id)?.is_none() {
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
                let temp = state.studio.settings.data_dir.join(format!(
                    ".upload-{}{}",
                    Uuid::new_v4(),
                    suffix
                ));
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

#[derive(Deserialize)]
struct RenameProfileBody {
    style_name: String,
}

async fn rename_profile(
    State(state): State<AppState>,
    jar: CookieJar,
    AxumPath(profile_id): AxumPath<String>,
    Json(body): Json<RenameProfileBody>,
) -> AppResult<Response> {
    require_auth(&state, &jar)?;
    match state.studio.rename_profile(&profile_id, &body.style_name) {
        Ok(v) => Ok((StatusCode::OK, Json(v)).into_response()),
        Err(e) => {
            if let Some(StudioError::ProfileNotFound) = e.downcast_ref() {
                Err(AppError::not_found("Profile not found"))
            } else if let Some(StudioError::NameConflict) = e.downcast_ref() {
                Err(AppError::api(
                    StatusCode::CONFLICT,
                    "name_conflict",
                    "This speaker already has a profile with that style name",
                ))
            } else {
                Err(AppError::invalid_request(e.to_string()))
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
    if !target_text_is_valid(text) {
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
        Err(e) => Err(subtitle_error(&e)),
    }
}

fn subtitle_error(e: &anyhow::Error) -> AppError {
    let msg = e.to_string();
    if msg.contains("must be inside") {
        AppError::api(StatusCode::UNPROCESSABLE_ENTITY, "invalid_video_path", msg)
    } else {
        tracing::warn!(error = %msg, "Subtitle extraction failed");
        AppError::api(
            StatusCode::SERVICE_UNAVAILABLE,
            "subtitle_extraction_failed",
            msg,
        )
    }
}

async fn video_subtitles_upload(
    State(state): State<AppState>,
    jar: CookieJar,
    mut multipart: Multipart,
) -> AppResult<Json<Value>> {
    require_auth(&state, &jar)?;
    let mut upload_path: Option<PathBuf> = None;

    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::invalid_request(e.to_string()))?
    {
        if field.name() != Some("video") {
            continue;
        }
        let original_name = field.file_name().unwrap_or("video.mp4").to_string();
        let suffix = Path::new(&original_name)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!(".{}", e.to_ascii_lowercase()))
            .unwrap_or_else(|| ".mp4".into());
        if !video_extension_allowed(Path::new(&format!("x{suffix}"))) {
            return Err(AppError::api(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported_video",
                "Unsupported video format",
            ));
        }
        let temp = state.studio.settings.data_dir.join(format!(
            ".upload-video-{}{}",
            Uuid::new_v4(),
            suffix
        ));
        let mut file = tokio::fs::File::create(&temp)
            .await
            .map_err(|e| AppError::Internal(e.into()))?;
        let mut total: u64 = 0;
        let write_result: AppResult<()> = async {
            while let Some(chunk) = field
                .chunk()
                .await
                .map_err(|e| AppError::invalid_request(e.to_string()))?
            {
                total += chunk.len() as u64;
                if total > MAX_VIDEO_UPLOAD_BYTES {
                    return Err(AppError::api(
                        StatusCode::PAYLOAD_TOO_LARGE,
                        "upload_too_large",
                        "Video exceeds 2 GiB",
                    ));
                }
                tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
                    .await
                    .map_err(|e| AppError::Internal(e.into()))?;
            }
            Ok(())
        }
        .await;
        if let Err(e) = write_result {
            let _ = fs::remove_file(&temp);
            return Err(e);
        }
        upload_path = Some(temp);
    }

    let Some(upload_path) = upload_path else {
        return Err(AppError::invalid_request("Video file is required"));
    };

    let studio = state.studio.clone();
    let path_c = upload_path.clone();
    let result = tokio::task::spawn_blocking(move || studio.extract_subtitles_from_upload(&path_c))
        .await
        .map_err(|e| AppError::Internal(e.into()))?;
    let _ = fs::remove_file(&upload_path);

    match result {
        Ok(v) => Ok(Json(v)),
        Err(e) => Err(subtitle_error(&e)),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with_origin(origin: &'static str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::ORIGIN, origin.parse().unwrap());
        headers
    }

    #[test]
    fn webauthn_accepts_localhost_http_origin() {
        assert!(webauthn_from_origin(&headers_with_origin("http://localhost:7860")).is_ok());
    }

    #[test]
    fn webauthn_accepts_https_domain_origin() {
        assert!(webauthn_from_origin(&headers_with_origin("https://video.example.com")).is_ok());
    }

    #[test]
    fn webauthn_rejects_ipv4_literal_with_actionable_error() {
        let error = webauthn_from_origin(&headers_with_origin("http://127.0.0.1:7860"))
            .expect_err("IP literals must be rejected");
        assert!(matches!(
            error,
            AppError::Api {
                code: "webauthn_ip_origin_unsupported",
                ..
            }
        ));
    }

    #[test]
    fn webauthn_rejects_ipv6_literal_with_actionable_error() {
        let error = webauthn_from_origin(&headers_with_origin("http://[::1]:7860"))
            .expect_err("IP literals must be rejected");
        assert!(matches!(
            error,
            AppError::Api {
                code: "webauthn_ip_origin_unsupported",
                ..
            }
        ));
    }

    #[test]
    fn webauthn_rejects_http_domain_origin() {
        assert!(webauthn_from_origin(&headers_with_origin("http://video.example.com")).is_err());
    }
}
