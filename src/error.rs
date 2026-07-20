use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use thiserror::Error;

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("{message}")]
    Api {
        code: &'static str,
        message: String,
        status: StatusCode,
    },
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl AppError {
    pub fn api(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self::Api {
            code,
            message: message.into(),
            status,
        }
    }

    pub fn unauthorized() -> Self {
        Self::api(
            StatusCode::UNAUTHORIZED,
            "authentication_required",
            "Please sign in",
        )
    }

    pub fn forbidden_origin() -> Self {
        Self::api(
            StatusCode::FORBIDDEN,
            "forbidden_origin",
            "Cross-origin request rejected",
        )
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::api(StatusCode::NOT_FOUND, "not_found", message)
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::api(StatusCode::UNPROCESSABLE_ENTITY, "invalid_request", message)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            Self::Api {
                code,
                message,
                status,
            } => (
                status,
                Json(json!({ "error": { "code": code, "message": message } })),
            )
                .into_response(),
            Self::Internal(err) => {
                tracing::error!(error = %err, "internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({
                        "error": {
                            "code": "internal_error",
                            "message": "Internal server error"
                        }
                    })),
                )
                    .into_response()
            }
        }
    }
}
