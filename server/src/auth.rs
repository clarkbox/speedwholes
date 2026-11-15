//! Admin authentication middleware for /config007/* endpoints.
//!
//! Uses session-based authentication:
//! - Login endpoint validates admin token and returns a session token
//! - Session tokens are stored in memory and cleared on server restart
//! - All other admin endpoints validate the session token

use axum::{
    body::Body,
    extract::State,
    http::{header, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use subtle::ConstantTimeEq;

use crate::config::AppState;

/// Login result with session token
pub struct LoginResult {
    pub session_token: String,
}

/// Validate admin token and create a session.
/// Returns a session token if the admin token is valid.
pub fn login(state: &AppState, admin_token: &str) -> Result<LoginResult, &'static str> {
    // Check if admin token is configured
    let expected_token = match &state.admin_token {
        Some(token) => token,
        None => {
            tracing::warn!("Login attempted but no admin token configured");
            return Err("Admin token not configured. Set admin_token in speedwholes.config.json.");
        }
    };

    // Constant-time comparison of provided token against configured admin token
    let is_valid: bool = expected_token
        .as_bytes()
        .ct_eq(admin_token.as_bytes())
        .into();

    if is_valid {
        // Create a new session and return the session token
        let session_token = state.sessions.create_session();
        Ok(LoginResult { session_token })
    } else {
        tracing::warn!("Invalid admin token provided during login");
        Err("Invalid admin token")
    }
}

/// Logout - invalidate a session token.
pub fn logout(state: &AppState, session_token: &str) -> bool {
    state.sessions.remove_session(session_token)
}

/// Middleware that validates session tokens for protected endpoints.
///
/// Authentication method:
/// - Bearer token in Authorization header - validated against in-memory session store
///
/// Sessions are stored in memory and cleared on server restart.
pub async fn require_admin_auth(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    // If no admin token is configured, deny access (can't create sessions without it)
    if state.admin_token.is_none() {
        tracing::warn!(
            path = %request.uri().path(),
            "Admin endpoint accessed but no admin token configured - access denied"
        );
        return (
            StatusCode::UNAUTHORIZED,
            "Admin token not configured. Set admin_token in speedwholes.config.json.",
        )
            .into_response();
    }

    // Extract Bearer token from Authorization header
    let auth_header = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());

    if let Some(header) = auth_header {
        if let Some(provided_token) = header.strip_prefix("Bearer ") {
            // Validate against session store
            if state.sessions.validate_session(provided_token) {
                return next.run(request).await;
            } else {
                tracing::debug!(
                    path = %request.uri().path(),
                    "Invalid or expired session token"
                );
                return (StatusCode::UNAUTHORIZED, "Invalid or expired session").into_response();
            }
        }
    }

    // No valid authentication found
    tracing::debug!(
        path = %request.uri().path(),
        "No valid Authorization header"
    );
    (
        StatusCode::UNAUTHORIZED,
        "Authentication required. Use Authorization: Bearer <session_token>",
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constant_time_comparison() {
        let token1 = "secret-token-12345";
        let token2 = "secret-token-12345";
        let token3 = "wrong-token-67890";

        // Same tokens should match
        let same: bool = token1.as_bytes().ct_eq(token2.as_bytes()).into();
        assert!(same);

        // Different tokens should not match
        let different: bool = token1.as_bytes().ct_eq(token3.as_bytes()).into();
        assert!(!different);
    }
}
