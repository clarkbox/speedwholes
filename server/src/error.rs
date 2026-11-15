use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::time::Duration;
use thiserror::Error;
use tower_http::classify::ServerErrorsFailureClass;
use tracing::Span;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Invalid fit mode: {0}. Valid modes: contain, cover, fill")]
    InvalidFitMode(String),

    #[error("Invalid output format: {0}. Valid formats: jpg, png, webp, gif")]
    InvalidFormat(String),

    #[error("Dimension exceeds maximum allowed: {0} > {1}")]
    DimensionTooLarge(u32, u32),

    #[error("Image dimensions too large: {width}x{height} ({pixels} pixels) exceeds limit of {max_pixels}")]
    ImageDimensionsTooLarge {
        width: usize,
        height: usize,
        pixels: u64,
        max_pixels: u64,
    },

    /// Upstream fetch failed. `message` is for logs only; the HTTP body is generic.
    #[error("{message}")]
    FetchError {
        message: String,
        status: Option<u16>,
    },

    #[error("Failed to decode image: {0}")]
    DecodeError(String),

    #[error("Failed to encode image: {0}")]
    EncodeError(String),

    #[error("Cache error: {0}")]
    CacheError(String),

    #[error("Signature required but not provided")]
    SignatureRequired,

    #[error("Invalid or expired signature")]
    InvalidSignature,

    #[error("Signing key not configured for this subdomain")]
    SigningNotConfigured,

    #[error("Source domain not allowed: {0}")]
    SourceNotAllowed(String),

    #[error("Source exceeds the maximum download size of {0} bytes")]
    SourceTooLarge(u64),

    #[error("Bypass mode is disabled for this subdomain")]
    BypassDisabled,

    #[error("Unknown host")]
    UnknownHost,
}

impl AppError {
    /// Get an error code for structured logging
    pub fn error_code(&self) -> &'static str {
        match self {
            AppError::InvalidFitMode(_) => "INVALID_FIT_MODE",
            AppError::InvalidFormat(_) => "INVALID_FORMAT",
            AppError::DimensionTooLarge(_, _) => "DIMENSION_TOO_LARGE",
            AppError::ImageDimensionsTooLarge { .. } => "IMAGE_DIMENSIONS_TOO_LARGE",
            AppError::FetchError { .. } => "FETCH_ERROR",
            AppError::DecodeError(_) => "DECODE_ERROR",
            AppError::EncodeError(_) => "ENCODE_ERROR",
            AppError::CacheError(_) => "CACHE_ERROR",
            AppError::SignatureRequired => "SIGNATURE_REQUIRED",
            AppError::InvalidSignature => "INVALID_SIGNATURE",
            AppError::SigningNotConfigured => "SIGNING_NOT_CONFIGURED",
            AppError::SourceNotAllowed(_) => "SOURCE_NOT_ALLOWED",
            AppError::SourceTooLarge(_) => "SOURCE_TOO_LARGE",
            AppError::BypassDisabled => "BYPASS_DISABLED",
            AppError::UnknownHost => "UNKNOWN_HOST",
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match &self {
            AppError::InvalidFitMode(_)
            | AppError::InvalidFormat(_)
            | AppError::DimensionTooLarge(_, _) => StatusCode::BAD_REQUEST,
            AppError::ImageDimensionsTooLarge { .. }
            | AppError::DecodeError(_)
            | AppError::SourceTooLarge(_) => StatusCode::UNPROCESSABLE_ENTITY,
            AppError::FetchError { .. } => StatusCode::BAD_GATEWAY,
            AppError::EncodeError(_) | AppError::CacheError(_) | AppError::SigningNotConfigured => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
            AppError::SignatureRequired
            | AppError::InvalidSignature
            | AppError::SourceNotAllowed(_)
            | AppError::BypassDisabled => StatusCode::FORBIDDEN,
            AppError::UnknownHost => StatusCode::NOT_FOUND,
        };

        let body = match &self {
            AppError::FetchError {
                status: upstream, ..
            } => {
                let mut body = "[FETCH_ERROR] upstream fetch failed".to_string();
                if let Some(code) = upstream {
                    body.push_str(&format!(" (HTTP {code})"));
                }
                body
            }
            _ => format!("[{}] {}", self.error_code(), self),
        };

        tracing::error!(
            error_code = self.error_code(),
            status_code = status.as_u16(),
            error_message = %self,
            "Request failed"
        );

        (status, body).into_response()
    }
}

pub type Result<T> = std::result::Result<T, AppError>;

/// Custom on_failure handler for TraceLayer with detailed error logging
pub fn on_failure(failure_class: ServerErrorsFailureClass, latency: Duration, _span: &Span) {
    match failure_class {
        ServerErrorsFailureClass::StatusCode(status) => {
            tracing::error!(
                status_code = status.as_u16(),
                status_text = %status.canonical_reason().unwrap_or("Unknown"),
                latency_ms = latency.as_millis() as u64,
                "Response failed with status code"
            );
        }
        ServerErrorsFailureClass::Error(err) => {
            tracing::error!(
                error = %err,
                latency_ms = latency.as_millis() as u64,
                "Response failed with error"
            );
        }
    }
}
