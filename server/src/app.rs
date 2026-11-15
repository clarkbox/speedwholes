//! HTTP router, middleware, and shared layers.

use axum::http::{header, HeaderValue};
use axum::{
    middleware,
    response::IntoResponse,
    routing::{delete, get, patch, post, put},
    Router,
};
use tower_http::{
    catch_panic::CatchPanicLayer,
    cors::{Any, CorsLayer},
    set_header::SetResponseHeaderLayer,
    trace::{DefaultMakeSpan, DefaultOnRequest, DefaultOnResponse, TraceLayer},
};
use tracing::Level;

use crate::auth;
use crate::config::AppState;
use crate::error;
use crate::handlers;

/// Panic response for `CatchPanicLayer`. The payload is logged and is not written to the body.
pub fn panic_response(
    panic_info: Box<dyn std::any::Any + Send + 'static>,
) -> axum::response::Response {
    let message = if let Some(s) = panic_info.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = panic_info.downcast_ref::<String>() {
        s.clone()
    } else {
        "Unknown panic".to_string()
    };

    tracing::error!(
        panic_message = %message,
        error_code = "HANDLER_PANIC",
        "Handler panicked"
    );

    (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        "[HANDLER_PANIC] internal server error",
    )
        .into_response()
}

/// Build the application router. Behaviour matches the previous inline setup in `main`.
pub fn build_router(state: AppState) -> Router {
    let trace_layer = TraceLayer::new_for_http()
        .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
        .on_request(DefaultOnRequest::new().level(Level::INFO))
        .on_response(DefaultOnResponse::new().level(Level::INFO))
        .on_failure(error::on_failure);

    let catch_panic_layer = CatchPanicLayer::custom(panic_response);

    // CORS configuration - allow all origins for maximum compatibility
    // Security is enforced via signing keys and admin authentication, not CORS
    // This enables crossOrigin='anonymous' for canvas-based image editors
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::PUT,
            axum::http::Method::PATCH,
            axum::http::Method::DELETE,
            axum::http::Method::OPTIONS,
        ])
        .allow_headers(Any);

    // Admin API routes protected by session token authentication
    let protected_admin_routes = Router::new()
        .route("/json", get(handlers::config_json_get))
        .route("/stats", get(handlers::stats_json))
        .route("/stats/sse", get(handlers::stats_sse))
        .route("/disk-stats", get(handlers::disk_stats))
        .route(
            "/resize-stats",
            get(handlers::resize_stats_get).delete(handlers::resize_stats_reset),
        )
        .route(
            "/resize-stats/:subdomain",
            delete(handlers::resize_stats_reset_subdomain),
        )
        .route("/logs", get(handlers::logs_get))
        .route(
            "/subdomains",
            get(handlers::subdomains_get).post(handlers::subdomains_post),
        )
        .route(
            "/subdomain/:name",
            post(handlers::subdomain_save).delete(handlers::subdomain_delete),
        )
        .route(
            "/subdomain/:name/params",
            patch(handlers::subdomain_params_patch),
        )
        .route(
            "/subdomain/:name/template/:template_name",
            put(handlers::subdomain_template_put).delete(handlers::subdomain_template_delete),
        )
        .route(
            "/subdomain/:name/cache/resize",
            delete(handlers::subdomain_cache_clear_resize),
        )
        .route(
            "/subdomain/:name/cache/source",
            delete(handlers::subdomain_cache_clear_source),
        )
        .route("/sign", post(handlers::sign_url))
        .route("/logout", post(handlers::logout_handler))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_admin_auth,
        ));

    // Public auth routes (login doesn't require session)
    let admin_routes = Router::new()
        .route("/login", post(handlers::login_handler))
        .merge(protected_admin_routes);

    let csp = SetResponseHeaderLayer::if_not_present(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; sandbox"),
    );
    let nosniff = SetResponseHeaderLayer::if_not_present(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );

    Router::new()
        .route("/", get(handlers::root_docs))
        .route("/health", get(handlers::health))
        // Mount admin routes under /config007
        .nest("/config007", admin_routes)
        // Main resize endpoint with path-based parameters
        // Format: /resize/w/{width}/h/{height}/fit/{mode}/{url}
        .route(
            "/resize",
            get(handlers::resize_no_params).layer(csp.clone()),
        )
        .route(
            "/resize/",
            get(handlers::resize_no_params).layer(csp.clone()),
        )
        .route("/resize/*rest", get(handlers::resize_handler).layer(csp))
        .fallback(handlers::not_found)
        .layer(cors)
        .layer(catch_panic_layer)
        .layer(trace_layer)
        .layer(nosniff)
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn panic_response_hides_the_payload() {
        let payload = "secret-panic-payload-xyz";
        let response = panic_response(Box::new(payload));
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains(payload));
        assert!(text.contains("[HANDLER_PANIC] internal server error"));
    }
}
