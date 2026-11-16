use crate::auth;
use crate::cache::{
    clear_subdomain_cache, extension_from_content_type, get_disk_stats, mime_for_extension,
    DiskCache, DiskStatsResponse,
};
use crate::config::{AppState, ConfigResponse, StatsSnapshot, SubdomainConfig};
use crate::error::{AppError, Result};
use crate::resize::{
    decode_image_exact, encode_image, fetch_image, resize_image_exact, FitMode, OutputFormat,
};
use crate::source_filter;
use axum::{
    extract::{OriginalUri, State},
    http::{header, HeaderMap, StatusCode, Uri},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    Json,
};
use futures::stream::{self, Stream, StreamExt};
use serde::Deserialize;
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

/// Build a JSON error response with the given status code and message.
fn json_error(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        Json(serde_json::json!({
            "error": message.into()
        })),
    )
        .into_response()
}

/// Format a duration in human-readable form
fn format_duration(d: std::time::Duration) -> String {
    let micros = d.as_micros();
    if micros < 1_000 {
        format!("{}µs", micros)
    } else if micros < 1_000_000 {
        format!("{:.1}ms", micros as f64 / 1_000.0)
    } else {
        format!("{:.2}s", micros as f64 / 1_000_000.0)
    }
}

/// Source mode - either a direct URL or parts to interpolate into a template
#[derive(Debug, Clone)]
enum SourceMode {
    /// Full URL provided directly (/u/ prefix or legacy http(s):// detection)
    Url(String),
    /// Parts to interpolate into subdomain's source_template (/p/ prefix)
    /// e.g., /p/jpg/path/filename -> ["jpg", "path", "filename"]
    Parts(Vec<String>),
}

/// Parsed resize parameters from URL
struct ResizeParams {
    /// Width (optional if using template or bypass mode)
    width: Option<u32>,
    /// Height (optional if using template or bypass mode)
    height: Option<u32>,
    fit: Option<FitMode>,
    quality: Option<u8>,
    format: Option<OutputFormat>,
    nocache: bool,
    /// Bypass resize - serve source image directly
    bypass: bool,
    /// Source - either direct URL or parts for template interpolation
    source: SourceMode,
    /// URL signature (if provided via `s` param)
    signature: Option<String>,
    /// Canonical path used for signature verification (sorted params + URL, excludes `s`)
    signing_path: String,
    /// Template name (if using /t/name/ mode)
    template_name: Option<String>,
}

/// Parse resize parameters from raw path
/// Format: /resize/{key}/{value}/{key}/{value}/.../u/https://... OR /p/part0/part1/...
///
/// Source modes:
/// - /u/ prefix: URL mode - everything after /u/ is the literal source URL
/// - /p/ prefix: Parts mode - slash-delimited parts for template interpolation
/// - Legacy: bare http:// or https:// (backwards compatible, treated as URL mode)
///
/// Template mode:
/// - /t/name/ prefix: Use a named template for resize params
///   When using template mode, w/h/fit/q/f params are optional (come from template)
///
/// Examples:
///   /resize/w/300/h/200/u/https://example.com/img.jpg
///   /resize/w/300/h/200/p/jpg/path/filename
///   /resize/w/300/h/200/s/{sig}/u/https://example.com/img.jpg
///   /resize/w/300/h/200/https://example.com/img.jpg (legacy)
///   /resize/t/thumbnail/u/https://example.com/img.jpg (template mode)
///   /resize/t/hero/p/jpg/photos/banner (template + parts mode)
fn parse_resize_params(raw_path: &str) -> Result<ResizeParams> {
    // Strip /resize/ prefix
    let prefix = "/resize/";
    if !raw_path.starts_with(prefix) {
        return Err(AppError::InvalidFitMode(
            "Path must start with /resize/".to_string(),
        ));
    }
    // Trim any leading slashes to handle double-slash typos like /resize//t/...
    let after_prefix = raw_path[prefix.len()..].trim_start_matches('/');

    // Check for template mode: /t/name/ at the start
    let (template_name, after_template) = if let Some(rest) = after_prefix.strip_prefix("t/") {
        // Extract template name (everything until next /)
        if let Some(end_idx) = rest.find('/') {
            let name = &rest[..end_idx];
            if name.is_empty() {
                return Err(AppError::InvalidFitMode(
                    "Template name cannot be empty".to_string(),
                ));
            }
            (Some(name.to_string()), &rest[end_idx + 1..])
        } else {
            return Err(AppError::InvalidFitMode(
                "Template mode requires source: /t/name/u/URL or /t/name/p/parts".to_string(),
            ));
        }
    } else {
        (None, after_prefix)
    };

    // Find source mode marker: /u/ or /p/, or fall back to legacy http(s):// detection
    let (params_section, source, source_for_signing) = if let Some(idx) = after_template.find("/u/")
    {
        // URL mode: everything after /u/ is the literal URL
        let source_url = &after_template[idx + 3..]; // skip "/u/"
        (
            &after_template[..idx],
            SourceMode::Url(source_url.to_string()),
            source_url,
        )
    } else if let Some(source_url) = after_template.strip_prefix("u/") {
        // URL mode at start (after template): /t/name/u/URL
        ("", SourceMode::Url(source_url.to_string()), source_url)
    } else if let Some(idx) = after_template.find("/p/") {
        // Parts mode: split remaining path by /
        let parts_str = &after_template[idx + 3..]; // skip "/p/"
                                                    // Strip querystring from parts - it's for cache busting the resize URL, not part of source
        let parts_str_no_query = parts_str.split('?').next().unwrap_or(parts_str);
        let parts: Vec<String> = parts_str_no_query
            .split('/')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        if parts.is_empty() {
            return Err(AppError::InvalidFitMode(
                "Parts mode (/p/) requires at least one part".to_string(),
            ));
        }
        // For signing, use the parts string without querystring
        (
            &after_template[..idx],
            SourceMode::Parts(parts),
            parts_str_no_query,
        )
    } else if let Some(parts_str) = after_template.strip_prefix("p/") {
        // Parts mode at start (after template): /t/name/p/parts
        // Strip querystring from parts - it's for cache busting the resize URL, not part of source
        let parts_str_no_query = parts_str.split('?').next().unwrap_or(parts_str);
        let parts: Vec<String> = parts_str_no_query
            .split('/')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        if parts.is_empty() {
            return Err(AppError::InvalidFitMode(
                "Parts mode (/p/) requires at least one part".to_string(),
            ));
        }
        ("", SourceMode::Parts(parts), parts_str_no_query)
    } else {
        // Legacy: look for http:// or https://
        let url_start = after_template
            .find("http://")
            .or_else(|| after_template.find("https://"))
            .ok_or_else(|| {
                AppError::InvalidFitMode(
                    "No source found. Use /u/URL or /p/part1/part2/... or legacy http(s)://"
                        .to_string(),
                )
            })?;
        let source_url = &after_template[url_start..];
        (
            &after_template[..url_start],
            SourceMode::Url(source_url.to_string()),
            source_url,
        )
    };

    // Parse params section as key/value pairs
    // "w/300/h/200/fit/cover/" -> ["w", "300", "h", "200", "fit", "cover"]
    let parts: Vec<&str> = params_section
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();

    // Must have even number of parts (key/value pairs)
    if !parts.len().is_multiple_of(2) {
        return Err(AppError::InvalidFitMode(format!(
            "Parameters must be in key/value pairs, got {} parts",
            parts.len()
        )));
    }

    // Build HashMap from pairs
    let mut params: HashMap<&str, &str> = HashMap::new();
    for chunk in parts.chunks(2) {
        params.insert(chunk[0], chunk[1]);
    }

    // Extract signature if present (and remove from params for signing path calculation)
    let signature = params.remove("s").map(|s| s.to_string());

    // Build canonical signing path (sorted params excluding 's', plus source)
    // Include template name in signing path if present
    let signing_path = if let Some(ref tmpl) = template_name {
        format!(
            "t/{}/{}",
            tmpl,
            crate::signing::build_canonical_path(
                params.iter().map(|(k, v)| (*k, *v)),
                source_for_signing,
            )
        )
    } else {
        crate::signing::build_canonical_path(
            params.iter().map(|(k, v)| (*k, *v)),
            source_for_signing,
        )
    };

    // Width and height are optional - may come from template or default template
    let width: Option<u32> = params
        .get("w")
        .map(|v| {
            v.parse()
                .map_err(|_| AppError::InvalidFitMode("Invalid width value".to_string()))
        })
        .transpose()?;

    let height: Option<u32> = params
        .get("h")
        .map(|v| {
            v.parse()
                .map_err(|_| AppError::InvalidFitMode("Invalid height value".to_string()))
        })
        .transpose()?;

    // Note: width/height validation happens in the handler where we can check for default templates

    // Extract optional params
    let fit = params
        .get("fit")
        .map(|s| FitMode::from_str(s))
        .transpose()?;

    // Parse quality (1-100)
    let quality: Option<u8> = params
        .get("q")
        .map(|v| v.parse::<u8>().unwrap_or(85).clamp(1, 100));

    // Parse output format (optional)
    let format = params
        .get("f")
        .map(|f| OutputFormat::from_str(f))
        .transpose()?;

    let nocache = params
        .get("nocache")
        .map(|v| *v == "1" || *v == "true")
        .unwrap_or(false);

    let bypass = params
        .get("b")
        .map(|v| *v == "1" || *v == "true")
        .unwrap_or(false);

    Ok(ResizeParams {
        width,
        height,
        fit,
        quality,
        format,
        nocache,
        bypass,
        source,
        signature,
        signing_path,
        template_name,
    })
}

/// Interpolate parts into a template string
/// Template: "https://example.com/{1}/{2}.{0}"
/// Parts: ["jpg", "path", "filename"]
/// Result: "https://example.com/path/filename.jpg"
/// How a resize request's source URL should be checked.
struct SourcePolicy {
    /// When true, any public host is allowed and `patterns` is ignored.
    allow_all: bool,
    patterns: Vec<String>,
}

/// Tenant allowlists win when they are non-empty. An empty tenant list is "not set"
/// and falls back to the global list. `allow_all_sources` applies only in that fallback.
fn resolve_source_policy(
    allow_all_sources: bool,
    global: &[String],
    tenant_sources: Option<&[String]>,
) -> SourcePolicy {
    if let Some(sources) = tenant_sources {
        if !sources.is_empty() {
            return SourcePolicy {
                allow_all: false,
                patterns: sources.to_vec(),
            };
        }
    }
    if allow_all_sources {
        return SourcePolicy {
            allow_all: true,
            patterns: Vec::new(),
        };
    }
    SourcePolicy {
        allow_all: false,
        patterns: global.to_vec(),
    }
}

fn part_is_unsafe(part: &str) -> bool {
    if part.is_empty() {
        return true;
    }
    let decoded = percent_encoding::percent_decode_str(part).decode_utf8_lossy();
    if decoded.contains('\\') || decoded.contains('#') || decoded.contains('@') {
        return true;
    }
    decoded
        .split('/')
        .any(|segment| segment == "." || segment == "..")
}

/// Interpolate parts into a template string.
/// Template: "https://example.com/{1}/{2}.{0}"
/// Parts: ["jpg", "path", "filename"]
/// Result: "https://example.com/path/filename.jpg"
///
/// `%2F` inside a part is kept. A part that is `.` or `..` after decoding,
/// or that contains `\`, `#`, or `@`, is rejected. When the template host has
/// no `{n}` placeholder, the result must keep that host.
fn interpolate_template(template: &str, parts: &[String]) -> Result<String> {
    if parts.iter().any(|part| part_is_unsafe(part)) {
        return Err(AppError::SourceNotAllowed("invalid path part".to_string()));
    }

    let mut result = template.to_string();
    for (i, part) in parts.iter().enumerate() {
        result = result.replace(&format!("{{{}}}", i), part);
    }

    if let Ok(template_url) = url::Url::parse(template) {
        let host_has_placeholder = template_url
            .host_str()
            .is_some_and(|host| host.contains('{'));
        if !host_has_placeholder {
            let result_url = url::Url::parse(&result)
                .map_err(|_| AppError::SourceNotAllowed("invalid interpolated url".to_string()))?;
            if result_url.host() != template_url.host() {
                return Err(AppError::SourceNotAllowed(
                    result_url.host_str().unwrap_or("unknown").to_string(),
                ));
            }
        }
    }

    Ok(result)
}

/// Health check endpoint
pub async fn health() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

/// Login request body
#[derive(Deserialize)]
pub struct LoginRequest {
    /// The admin token to authenticate with
    pub token: String,
}

/// Login handler - validates admin token and returns a session token.
///
/// POST /config007/login
/// Body: { "token": "your-admin-token" }
///
/// Returns: { "session_token": "uuid-session-token" }
///
/// The session token should be used for all subsequent admin API requests.
/// Sessions are stored in memory and cleared on server restart.
pub async fn login_handler(
    State(state): State<AppState>,
    Json(body): Json<LoginRequest>,
) -> Response {
    match auth::login(&state, &body.token) {
        Ok(result) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "session_token": result.session_token
            })),
        )
            .into_response(),
        Err(message) => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "error": message
            })),
        )
            .into_response(),
    }
}

/// Logout request - extracts session token from Authorization header
///
/// POST /config007/logout
/// Header: Authorization: Bearer <session_token>
///
/// Returns: { "success": true }
pub async fn logout_handler(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Response {
    // Extract session token from Authorization header
    let auth_header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());

    if let Some(header) = auth_header {
        if let Some(session_token) = header.strip_prefix("Bearer ") {
            let removed = auth::logout(&state, session_token);
            return (
                StatusCode::OK,
                Json(serde_json::json!({
                    "success": removed
                })),
            )
                .into_response();
        }
    }

    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({
            "error": "No session token provided"
        })),
    )
        .into_response()
}

/// 404 fallback handler
pub async fn not_found(uri: axum::http::Uri) -> impl IntoResponse {
    (StatusCode::NOT_FOUND, format!("404 - {} not found", uri))
}

/// Root route handler - show usage documentation
pub async fn root_docs() -> impl IntoResponse {
    (
        StatusCode::OK,
        "Speedwholes - Image Resize Service\n\n\
        Usage: /resize/{key}/{value}/.../{url}\n\n\
        Required params (unless using template or bypass mode):\n\
        - w: width in pixels\n\
        - h: height in pixels\n\n\
        Optional params:\n\
        - fit: cover (default), contain, fill\n\
        - q: quality 1-100 (default 85, affects JPEG/WebP)\n\
        - f: output format (jpg, png, webp, gif)\n\
        - nocache: 1 to bypass cache\n\
        - b: 1 to bypass resize (serve source image directly)\n\
        - s: URL signature (HMAC)\n\n\
        Bypass mode (b/1):\n\
        - Skips resize, serves cached source image directly\n\
        - No width/height required\n\
        - Source is still cached for future requests\n\
        - Example: /resize/b/1/u/https://example.com/image.jpg\n\n\
        Template mode (uses preset params from subdomain config):\n\
        - /resize/t/{template_name}/u/{url}\n\
        - /resize/t/{template_name}/p/{part0}/{part1}/...\n\n\
        Source modes:\n\
        - /u/{url}: Direct URL mode - full source URL follows\n\
        - /p/{parts}: Parts mode - uses source_template from subdomain config\n\
        - Legacy: bare http(s):// URL (backwards compatible)\n\n\
        Parts mode with source_template:\n\
        - Configure source_template in subdomain config (subdomainConfig/{name}.json)\n\
        - Template uses {0}, {1}, {2}... placeholders\n\
        - Parts in URL map to placeholders: /p/part0/part1/part2 -> {0}=part0, {1}=part1, {2}=part2\n\
        - Example template: \"https://cdn.example.com/{1}/{2}.{0}?token={3}\"\n\
        - With /p/jpg/photos/image123/mytoken resolves to:\n\
          https://cdn.example.com/photos/image123.jpg?token=mytoken\n\n\
        Examples:\n\
        - /resize/w/300/h/200/u/https://example.com/image.jpg\n\
        - /resize/w/300/h/200/fit/contain/u/https://example.com/image.jpg\n\
        - /resize/w/300/h/200/p/jpg/photos/image123 (with source_template in config)\n\
        - /resize/t/thumbnail/u/https://example.com/image.jpg\n\
        - /resize/t/hero/p/jpg/photos/banner\n\
        - /resize/b/1/u/https://example.com/image.jpg (bypass resize)\n\n\
        Admin API: /config007/\n\
        Health check: /health"
    )
}

/// Handler for /resize without params - show usage error
pub async fn resize_no_params() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        "404 NO IMAGE PARAMS SPECIFIED\n\n\
        Usage: /resize/{key}/{value}/.../{url}\n\n\
        Required params (unless using template or bypass mode):\n\
        - w: width in pixels\n\
        - h: height in pixels\n\n\
        Optional params:\n\
        - fit: cover (default), contain, fill\n\
        - q: quality 1-100 (default 85, affects JPEG/WebP)\n\
        - f: output format (jpg, png, webp, gif)\n\
        - nocache: 1 to bypass cache\n\
        - b: 1 to bypass resize (serve source image directly)\n\
        - s: URL signature (HMAC)\n\n\
        Bypass mode (b/1):\n\
        - Skips resize, serves cached source image directly\n\
        - No width/height required\n\
        - Source is still cached for future requests\n\
        - Example: /resize/b/1/u/https://example.com/image.jpg\n\n\
        Template mode (uses preset params from subdomain config):\n\
        - /resize/t/{template_name}/u/{url}\n\
        - /resize/t/{template_name}/p/{part0}/{part1}/...\n\n\
        Source modes:\n\
        - /u/{url}: Direct URL mode - full source URL follows\n\
        - /p/{parts}: Parts mode - uses source_template from subdomain config\n\
        - Legacy: bare http(s):// URL (backwards compatible)\n\n\
        Parts mode with source_template:\n\
        - Configure source_template in subdomain config (subdomainConfig/{name}.json)\n\
        - Template uses {0}, {1}, {2}... placeholders\n\
        - Parts in URL map to placeholders: /p/part0/part1/part2 -> {0}=part0, {1}=part1, {2}=part2\n\
        - Example template: \"https://cdn.example.com/{1}/{2}.{0}?token={3}\"\n\
        - With /p/jpg/photos/image123/mytoken resolves to:\n\
          https://cdn.example.com/photos/image123.jpg?token=mytoken\n\n\
        Examples:\n\
        - /resize/w/300/h/200/u/https://example.com/image.jpg\n\
        - /resize/w/300/h/200/fit/contain/u/https://example.com/image.jpg\n\
        - /resize/w/300/h/200/p/jpg/photos/image123 (with source_template in config)\n\
        - /resize/t/thumbnail/u/https://example.com/image.jpg\n\
        - /resize/t/hero/p/jpg/photos/banner\n\
        - /resize/b/1/u/https://example.com/image.jpg (bypass resize)"
    )
}

/// GET /config007/json - returns current config as JSON (read-only)
pub async fn config_json_get(State(state): State<AppState>) -> Json<ConfigResponse> {
    Json(ConfigResponse::from(state.config.as_ref()))
}

fn header_text(headers: &HeaderMap, name: header::HeaderName) -> Option<String> {
    let value = headers.get(name)?.to_str().ok()?.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn forwarded_host(value: &str) -> Option<String> {
    let first = value.split(',').next().unwrap_or(value);
    for param in first.split(';') {
        let Some(rest) = param.trim().strip_prefix("host=") else {
            continue;
        };
        let host = rest.trim().trim_matches('"');
        if !host.is_empty() {
            return Some(host.to_string());
        }
    }
    None
}

/// Host used for tenant selection. `Forwarded` and `X-Forwarded-Host` are
/// consulted only when `trust_forwarded_host` is set. Otherwise the `Host`
/// header is used, falling back to the URI authority (HTTP/2).
fn resolve_request_host(headers: &HeaderMap, uri: &Uri, trust_forwarded: bool) -> String {
    if trust_forwarded {
        if let Some(forwarded) = header_text(headers, header::FORWARDED) {
            if let Some(host) = forwarded_host(&forwarded) {
                return host;
            }
        }
        if let Some(forwarded) = headers
            .get("x-forwarded-host")
            .and_then(|v| v.to_str().ok())
        {
            let first = forwarded.split(',').next().unwrap_or(forwarded).trim();
            if !first.is_empty() {
                return first.to_string();
            }
        }
    }
    if let Some(host) = header_text(headers, header::HOST) {
        return host;
    }
    uri.authority()
        .map(|a| a.as_str().to_string())
        .unwrap_or_default()
}

/// Strip a port, including a bracketed IPv6 host such as `[::1]:3000`.
fn host_without_port(host: &str) -> &str {
    if let Some(rest) = host.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            return &rest[..end];
        }
    }
    if host.matches(':').count() == 1 {
        host.split(':').next().unwrap_or(host)
    } else {
        host
    }
}

/// Parse a tenant name from a host string.
/// `"foo.example.com"` -> `Some("foo")`. An apex is `None`.
/// An IP address or `localhost` is the candidate name `DEVLOCAL`.
fn parse_subdomain(host: &str) -> Option<String> {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    let host_without_port = host_without_port(&host);

    if host_without_port.parse::<std::net::IpAddr>().is_ok() || host_without_port == "localhost" {
        return Some("DEVLOCAL".to_string());
    }

    let parts: Vec<&str> = host_without_port
        .split('.')
        .filter(|s| !s.is_empty())
        .collect();
    if parts.len() >= 3 {
        Some(parts[..parts.len() - 2].join("."))
    } else {
        None
    }
}

fn warn_if_signing_misconfigured(config: &SubdomainConfig) {
    let missing = config
        .signing_key
        .as_deref()
        .map(|key| key.trim().is_empty())
        .unwrap_or(true);
    if config.signing_required && missing {
        tracing::warn!(
            tenant = %config.name,
            "signing_required is set but signing_key is missing; signed requests will fail"
        );
    }
}

/// Resize image endpoint
/// Flexible path format: /resize/{key}/{value}/{key}/{value}/.../u/URL or /p/part1/part2/...
///
/// Template mode:
///   /t/name/ - Use a named template from subdomain config for resize params
///   When using template, w/h/fit/q/f params come from the template (URL params ignored)
///
/// Source modes:
///   /u/ - URL mode: full source URL follows (e.g., /u/https://example.com/img.jpg)
///   /p/ - Parts mode: slash-delimited parts interpolated into subdomain's source_template
///   Legacy: bare http(s):// still works for backwards compatibility
///
/// Examples:
///   /resize/w/300/h/200/fit/cover/u/https://example.com/img.jpg
///   /resize/w/300/h/200/p/jpg/photos/image123 (with source_template in config)
///   /resize/w/300/h/200/https://example.com/img.jpg (legacy)
///   /resize/w/300/h/200/s/{signature}/u/https://example.com/img.jpg (signed URL)
///   /resize/t/thumbnail/u/https://example.com/img.jpg (template mode)
///   /resize/t/hero/p/jpg/photos/banner (template + parts mode)
///
/// Uses OriginalUri to preserve URL encoding (e.g., %2F in Firebase Storage paths)
pub async fn resize_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> Result<Response> {
    let request_start = std::time::Instant::now();
    let host = resolve_request_host(&headers, &uri, state.config.trust_forwarded_host);

    // Get raw path+query to preserve encoding like %2F
    let raw = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("");

    // Log the incoming request for debugging
    tracing::debug!(request_path = %raw, host = %host, "Processing resize request");

    // Parse the flexible key/value parameters
    let params = match parse_resize_params(raw) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(
                request_path = %raw,
                host = %host,
                error = %e,
                "Failed to parse resize parameters"
            );
            return Err(e);
        }
    };

    // A matching tenant file wins. IP and localhost are DEVLOCAL only when that
    // file exists. Anything else is unknown: 404, unless the policy is "global".
    let parsed = parse_subdomain(&host);
    let subdomain = {
        let configs = state.subdomain_configs.read().await;
        let matched = parsed.as_ref().and_then(|name| {
            configs
                .iter()
                .find(|config| config.name.eq_ignore_ascii_case(name))
                .map(|config| config.name.clone())
        });
        if let Some(name) = matched {
            Some(name)
        } else if state
            .config
            .unknown_host_policy
            .eq_ignore_ascii_case("global")
        {
            parsed.clone()
        } else {
            tracing::debug!(host = %host, parsed = ?parsed, "Unknown host rejected");
            return Err(AppError::UnknownHost);
        }
    };
    tracing::debug!("Request host: {}, subdomain: {:?}", host, subdomain);
    state.stats.record_subdomain(subdomain.clone());

    // Look up template if specified, and resolve final params
    // Returns: (width, height, fit, quality, format, source_template, template_name_used)
    // In bypass mode, dimensions don't matter - we skip resize entirely
    let bypass = params.bypass;
    let (width, height, fit, quality, format, template_source_template, template_name_used): (
        u32,
        u32,
        FitMode,
        u8,
        Option<OutputFormat>,
        Option<String>,
        Option<String>,
    ) = if bypass {
        // Bypass mode: skip dimension resolution, just need source_template for /p/ mode
        let subdomain_configs = state.subdomain_configs.read().await;
        let sub_config = subdomain
            .as_ref()
            .and_then(|sub_name| subdomain_configs.iter().find(|c| &c.name == sub_name));
        let source_template = sub_config.and_then(|c| c.source_template.clone());
        // Use placeholder values - they won't be used in bypass mode
        (0, 0, FitMode::Cover, 85, None, source_template, None)
    } else {
        let subdomain_configs = state.subdomain_configs.read().await;
        let sub_config = subdomain
            .as_ref()
            .and_then(|sub_name| subdomain_configs.iter().find(|c| &c.name == sub_name));

        if let Some(ref tmpl_name) = params.template_name {
            // Template mode: look up template and use its params
            // First try to find the requested template, fallback to default if not found
            let templates = sub_config.and_then(|c| c.templates.as_ref());
            let default_template_name = sub_config.and_then(|c| c.default_template.as_ref());

            let template = templates
                .and_then(|t| t.iter().find(|t| &t.name == tmpl_name))
                .or_else(|| {
                    // Fallback to default template if the requested one wasn't found
                    if let Some(default_name) = default_template_name {
                        tracing::debug!(
                            requested = %tmpl_name,
                            fallback = %default_name,
                            "Template not found, falling back to default"
                        );
                        templates.and_then(|t| t.iter().find(|t| &t.name == default_name))
                    } else {
                        None
                    }
                })
                .ok_or_else(|| {
                    AppError::InvalidFitMode(format!(
                        "Template '{}' not found in subdomain config",
                        tmpl_name
                    ))
                })?;

            tracing::debug!(
                template_name = %template.name,
                template = ?template,
                "Using resize template"
            );

            // Template params are required - at minimum width and height
            let width = template.width.ok_or_else(|| {
                AppError::InvalidFitMode(format!(
                    "Template '{}' missing required width",
                    template.name
                ))
            })?;
            let height = template.height.ok_or_else(|| {
                AppError::InvalidFitMode(format!(
                    "Template '{}' missing required height",
                    template.name
                ))
            })?;

            // Fit mode: template > default
            let fit = template
                .fit
                .as_ref()
                .map(|s| FitMode::from_str(s))
                .transpose()?
                .unwrap_or(FitMode::Cover);

            // Quality: template > default 85
            let quality = template.quality.unwrap_or(85).clamp(1, 100);

            // Format: template > none (auto-detect)
            let format = template
                .format
                .as_ref()
                .map(|f| OutputFormat::from_str(f))
                .transpose()?;

            (
                width,
                height,
                fit,
                quality,
                format,
                template.source_template.clone(),
                Some(template.name.clone()),
            )
        } else {
            // Non-template mode: check if default template should be used
            let templates = sub_config.and_then(|c| c.templates.as_ref());
            let default_template_name = sub_config.and_then(|c| c.default_template.as_ref());

            // Use default template if width/height are missing and a default is configured
            let use_default = (params.width.is_none() || params.height.is_none())
                && default_template_name.is_some();

            if use_default {
                let default_name = default_template_name.unwrap();
                if let Some(template) =
                    templates.and_then(|t| t.iter().find(|t| &t.name == default_name))
                {
                    tracing::debug!(
                        default_template = %default_name,
                        "No template specified and missing w/h, using default template"
                    );

                    let width = template.width.ok_or_else(|| {
                        AppError::InvalidFitMode(format!(
                            "Default template '{}' missing required width",
                            template.name
                        ))
                    })?;
                    let height = template.height.ok_or_else(|| {
                        AppError::InvalidFitMode(format!(
                            "Default template '{}' missing required height",
                            template.name
                        ))
                    })?;
                    let fit = template
                        .fit
                        .as_ref()
                        .map(|s| FitMode::from_str(s))
                        .transpose()?
                        .unwrap_or(FitMode::Cover);
                    let quality = template.quality.unwrap_or(85).clamp(1, 100);
                    let format = template
                        .format
                        .as_ref()
                        .map(|f| OutputFormat::from_str(f))
                        .transpose()?;

                    (
                        width,
                        height,
                        fit,
                        quality,
                        format,
                        template.source_template.clone(),
                        Some(template.name.clone()),
                    )
                } else {
                    // Default template not found, require URL params
                    let width = params.width.ok_or_else(|| {
                        AppError::InvalidFitMode("Missing required param: w (width)".to_string())
                    })?;
                    let height = params.height.ok_or_else(|| {
                        AppError::InvalidFitMode("Missing required param: h (height)".to_string())
                    })?;
                    let fit = params.fit.unwrap_or(FitMode::Cover);
                    let quality = params.quality.unwrap_or(85);
                    let format = params.format;

                    (width, height, fit, quality, format, None, None)
                }
            } else {
                // URL params are required if no default template
                let width = params.width.ok_or_else(|| {
                    AppError::InvalidFitMode("Missing required param: w (width)".to_string())
                })?;
                let height = params.height.ok_or_else(|| {
                    AppError::InvalidFitMode("Missing required param: h (height)".to_string())
                })?;
                let fit = params.fit.unwrap_or(FitMode::Cover);
                let quality = params.quality.unwrap_or(85);
                let format = params.format;

                (width, height, fit, quality, format, None, None)
            }
        }
    };

    let nocache = params.nocache;
    let fit_mode = fit.as_str().to_string();

    // Resolve full_url from SourceMode (needs subdomain config for Parts mode)
    let full_url = {
        let subdomain_configs = state.subdomain_configs.read().await;
        let sub_config = subdomain
            .as_ref()
            .and_then(|sub_name| subdomain_configs.iter().find(|c| &c.name == sub_name));

        match &params.source {
            SourceMode::Url(url) => url.clone(),
            SourceMode::Parts(parts) => {
                // For parts mode, use template's source_template if set, otherwise subdomain's
                let source_template = template_source_template.as_ref()
                    .or_else(|| sub_config.and_then(|c| c.source_template.as_ref()))
                    .ok_or_else(|| AppError::InvalidFitMode(
                        "Parts mode (/p/) requires source_template in template or subdomain config".to_string()
                    ))?;
                let url = interpolate_template(source_template, parts)?;
                tracing::debug!(
                    template = %source_template,
                    parts = ?parts,
                    resolved_url = %url,
                    "Resolved URL from template"
                );
                url
            }
        }
    };

    // Verify signature if subdomain has signing configured
    if let Some(ref sub_name) = subdomain {
        let subdomain_configs = state.subdomain_configs.read().await;
        if let Some(sub_config) = subdomain_configs.iter().find(|c| &c.name == sub_name) {
            let signing_key = sub_config
                .signing_key
                .as_deref()
                .filter(|key| !key.trim().is_empty());
            if sub_config.signing_required && signing_key.is_none() {
                tracing::warn!(
                    tenant = %sub_name,
                    "signing_required is set but signing_key is missing"
                );
                return Err(AppError::SigningNotConfigured);
            }
            if let Some(signing_key) = signing_key {
                // Subdomain has signing configured - verify signature
                match &params.signature {
                    Some(sig) => {
                        // Check signature cache first. Only valid results are stored.
                        let is_valid =
                            match state
                                .signature_cache
                                .get(sub_name, &params.signing_path, sig)
                            {
                                Some(cached_result) => {
                                    state.stats.record_signature_cache_hit();
                                    tracing::debug!(subdomain = %sub_name, "Signature cache hit");
                                    cached_result
                                }
                                None => {
                                    state.stats.record_signature_cache_miss();
                                    let valid = crate::signing::verify_signature(
                                        signing_key,
                                        &params.signing_path,
                                        sig,
                                    );
                                    if valid {
                                        state.signature_cache.put(
                                            sub_name,
                                            &params.signing_path,
                                            sig,
                                            true,
                                        );
                                    }
                                    valid
                                }
                            };

                        if !is_valid {
                            tracing::warn!(
                                subdomain = %sub_name,
                                signing_path = %params.signing_path,
                                "Invalid signature"
                            );
                            return Err(AppError::InvalidSignature);
                        }
                        tracing::debug!(subdomain = %sub_name, "Signature verified");
                    }
                    None if sub_config.signing_required => {
                        tracing::warn!(
                            subdomain = %sub_name,
                            "Signature required but not provided"
                        );
                        return Err(AppError::SignatureRequired);
                    }
                    None => {
                        // Signature not provided but not required - allow
                        tracing::debug!(subdomain = %sub_name, "No signature (not required)");
                    }
                }
            }
        }
    }

    // Check source URL allowlist (SSRF protection).
    // A non-empty tenant list is always enforced, even when allow_all_sources is set.
    let tenant_sources = {
        let subdomain_configs = state.subdomain_configs.read().await;
        subdomain.as_ref().and_then(|sub_name| {
            subdomain_configs
                .iter()
                .find(|c| &c.name == sub_name)
                .and_then(|c| c.allowed_sources.clone())
        })
    };
    let policy = resolve_source_policy(
        state.allow_all_sources(),
        &state.config.allowed_sources,
        tenant_sources.as_deref(),
    );
    let filter_policy = source_filter::SourcePolicy {
        allow_all: policy.allow_all,
        allow_private_networks: state.config.allow_private_networks,
    };
    if !source_filter::is_source_allowed(&full_url, &policy.patterns, &filter_policy) {
        let domain =
            source_filter::extract_domain(&full_url).unwrap_or_else(|| "unknown".to_string());
        tracing::warn!(
            source_url = %full_url,
            domain = %domain,
            subdomain = ?subdomain,
            patterns_count = policy.patterns.len(),
            allow_all = policy.allow_all,
            "Source URL not allowed"
        );
        return Err(AppError::SourceNotAllowed(domain));
    }
    let source_patterns = policy.patterns.clone();
    let validate_source =
        move |next: &str| source_filter::is_source_allowed(next, &source_patterns, &filter_policy);

    // Track request
    state.stats.record_request();

    // Effective limits are the tighter of the tenant cap and the global cap.
    let (max_width, max_height) = {
        let configs = state.subdomain_configs.read().await;
        let sub = subdomain
            .as_ref()
            .and_then(|name| configs.iter().find(|c| &c.name == name));
        let global_w = state.max_width();
        let global_h = state.max_height();
        (
            sub.and_then(|c| c.max_width)
                .map_or(global_w, |t| t.min(global_w)),
            sub.and_then(|c| c.max_height)
                .map_or(global_h, |t| t.min(global_h)),
        )
    };
    let cache_path = state.get_cache_path();

    // Validate dimensions
    if width > max_width {
        return Err(AppError::DimensionTooLarge(width, max_width));
    }
    if height > max_height {
        return Err(AppError::DimensionTooLarge(height, max_height));
    }

    // Determine the output extension (format override or preserve original)
    let url_extension = extension_from_content_type(None, &full_url);
    let output_extension = format.map(|f| f.to_extension()).unwrap_or(url_extension);

    tracing::info!(
        "Resize request: {}x{} fit={} q={} f={} url={}",
        width,
        height,
        fit_mode,
        quality,
        output_extension,
        full_url
    );

    // Initialize resize cache (with subdomain folder)
    let resize_cache_path = cache_path.join("resize");
    let cache =
        DiskCache::new_initialized(resize_cache_path.clone(), "resize", subdomain.as_deref())
            .await?;

    // Initialize source cache (with subdomain folder)
    let source_cache_path = cache_path.join("source");
    let source_cache =
        DiskCache::new_initialized(source_cache_path.clone(), "source", subdomain.as_deref())
            .await?;

    // Generate cache key (includes format and quality)
    let cache_key = DiskCache::generate_key(
        &full_url,
        width,
        height,
        fit.as_str(),
        quality,
        output_extension,
    );

    // Check cache (skip if nocache is set)
    if !nocache {
        match cache.get(&cache_key, output_extension).await {
            Ok(Some(cached_data)) => {
                state.stats.record_cache_hit();
                let total_time = request_start.elapsed();
                tracing::info!(
                    "Cache hit: {} - replied in {}",
                    cache_key,
                    format_duration(total_time)
                );
                return image_response(
                    cached_data,
                    output_extension,
                    url_extension,
                    quality,
                    ImageMetrics::cache_hit(total_time),
                    false,
                );
            }
            Ok(None) => {
                // Cache miss, continue to fetch
            }
            Err(e) => {
                tracing::warn!(
                    cache_key = %cache_key,
                    error = %e,
                    "Cache read error, proceeding without cache"
                );
                // Continue without cache rather than failing the request
            }
        }
    }

    if bypass {
        let configs = state.subdomain_configs.read().await;
        let bypass_disabled = subdomain.as_ref().and_then(|name| {
            configs
                .iter()
                .find(|config| &config.name == name)
                .and_then(|config| config.allow_bypass)
        }) == Some(false);
        if bypass_disabled {
            return Err(AppError::BypassDisabled);
        }
    }

    // Identical uncached requests share one fetch and resize. Bypass and
    // nocache do not join: bypass is a different response, and nocache asks
    // for a fresh one. A failed leader publishes the error so waiters do not
    // stampede the origin. A dropped leader (the client disconnected) frees
    // the key and one waiter takes over. Takeovers are bounded.
    let mut flight_leader = None;
    if !nocache && !bypass {
        let flight_key = format!(
            "{}:{cache_key}",
            subdomain
                .as_deref()
                .unwrap_or(crate::cache::DEFAULT_SUBDOMAIN)
        );
        for attempt in 0..INFLIGHT_TAKEOVERS {
            match state.inflight.join(&flight_key) {
                crate::inflight::Join::Leader(leader) => {
                    flight_leader = Some(leader);
                    break;
                }
                crate::inflight::Join::Follower(follower) => match follower.wait().await {
                    Ok(shared) => {
                        return coalesced_response(shared, quality, request_start.elapsed());
                    }
                    Err(crate::inflight::WaitError::Failed(err)) => return Err((*err).clone()),
                    Err(crate::inflight::WaitError::Cancelled) => {
                        if attempt + 1 == INFLIGHT_TAKEOVERS {
                            tracing::warn!(
                                flight_key = %flight_key,
                                "in-flight leaders kept disconnecting; this request will run on its own"
                            );
                        }
                    }
                },
            }
        }
    }

    // Generate source cache key (just URL hash)
    let source_cache_key = DiskCache::generate_source_key(&full_url);

    // Try source cache first to avoid network fetch
    let fetch_start = std::time::Instant::now();
    let (image_data, content_type, source_cache_hit) = {
        // Check source cache
        match source_cache.get(&source_cache_key, "bin").await {
            Ok(Some(cached_data)) => {
                state.stats.record_source_cache_hit();
                tracing::debug!("Source cache hit: {}", source_cache_key);
                let content_type = Some(
                    mime_for_extension(noted(&mut flight_leader, require_image(&cached_data))?)
                        .to_string(),
                );
                (cached_data, content_type, true)
            }
            Ok(None) => {
                state.stats.record_source_cache_miss();
                // Source cache miss - fetch from network
                match fetch_image(
                    &state.http_client,
                    &full_url,
                    state.config.source_compression,
                    &validate_source,
                    state.config.max_source_bytes,
                )
                .await
                {
                    Ok((data, _ct)) => {
                        let content_type = Some(
                            mime_for_extension(noted(&mut flight_leader, require_image(&data))?)
                                .to_string(),
                        );
                        // Store in source cache only after the bytes are a supported image.
                        let subdomain_clone = subdomain.clone();
                        let source_cache_clone =
                            DiskCache::new(source_cache_path.clone(), subdomain_clone.as_deref());
                        let source_key_clone = source_cache_key.clone();
                        let data_clone = data.clone();
                        tokio::spawn(async move {
                            if let Err(e) = source_cache_clone
                                .put(&source_key_clone, "bin", &data_clone)
                                .await
                            {
                                tracing::error!("Failed to cache source image: {}", e);
                            }
                        });
                        (data, content_type, false)
                    }
                    Err(e) => {
                        tracing::error!(
                            source_url = %full_url,
                            width = width,
                            height = height,
                            fit_mode = %fit_mode,
                            error = %e,
                            "Failed to fetch source image"
                        );
                        return Err(fail_inflight(&mut flight_leader, e));
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    source_cache_key = %source_cache_key,
                    error = %e,
                    "Source cache read error, fetching from network"
                );
                state.stats.record_source_cache_miss();
                // Fetch from network on cache error
                match fetch_image(
                    &state.http_client,
                    &full_url,
                    state.config.source_compression,
                    &validate_source,
                    state.config.max_source_bytes,
                )
                .await
                {
                    Ok((data, _ct)) => {
                        let content_type = Some(
                            mime_for_extension(noted(&mut flight_leader, require_image(&data))?)
                                .to_string(),
                        );
                        (data, content_type, false)
                    }
                    Err(e) => {
                        tracing::error!(
                            source_url = %full_url,
                            error = %e,
                            "Failed to fetch source image"
                        );
                        return Err(fail_inflight(&mut flight_leader, e));
                    }
                }
            }
        }
    };
    let fetch_time = fetch_start.elapsed();

    if source_cache_hit {
        tracing::info!(
            "Source cache hit for {} - loaded in {}",
            full_url,
            format_duration(fetch_time)
        );
    }

    // Bypass mode: return source image directly without resize
    if bypass {
        noted(
            &mut flight_leader,
            crate::resize::check_image_dimensions(&image_data, state.max_source_pixels()),
        )?;
        let total_time = request_start.elapsed();
        let original_extension = extension_from_content_type(content_type.as_deref(), &full_url);
        tracing::info!(
            "Bypass: {} ({} bytes) - fetched in {}, replied in {}",
            full_url,
            image_data.len(),
            format_duration(fetch_time),
            format_duration(total_time)
        );
        return bypass_image_response(
            image_data,
            original_extension,
            total_time,
            fetch_time,
            source_cache_hit,
            nocache,
        );
    }

    // Determine original format from content type
    let original_extension = extension_from_content_type(content_type.as_deref(), &full_url);

    // Final output extension: use format override if specified, otherwise original
    let final_output_extension = format
        .map(|f| f.to_extension())
        .unwrap_or(original_extension);

    // Regenerate cache key with correct original extension if it differs from URL guess
    // (only matters when no format override is specified)
    let final_cache_key = if format.is_none() && original_extension != url_extension {
        DiskCache::generate_key(
            &full_url,
            width,
            height,
            fit.as_str(),
            quality,
            final_output_extension,
        )
    } else {
        cache_key.clone()
    };

    // Check cache again with correct extension (in case URL extension was wrong and no format override)
    if !nocache && final_cache_key != cache_key {
        match cache.get(&final_cache_key, final_output_extension).await {
            Ok(Some(cached_data)) => {
                state.stats.record_cache_hit();
                let total_time = request_start.elapsed();
                tracing::info!(
                    "Cache hit: {} - replied in {}",
                    final_cache_key,
                    format_duration(total_time)
                );
                if let Some(leader) = flight_leader.as_ref() {
                    leader.publish(Arc::new(crate::inflight::SharedImage {
                        data: cached_data.clone(),
                        output_extension: final_output_extension,
                        original_extension,
                    }));
                }
                return image_response(
                    cached_data,
                    final_output_extension,
                    original_extension,
                    quality,
                    ImageMetrics::cache_hit(total_time),
                    false,
                );
            }
            Ok(None) => {
                // Cache miss, continue
            }
            Err(e) => {
                tracing::warn!(
                    cache_key = %final_cache_key,
                    extension = final_output_extension,
                    error = %e,
                    "Cache read error (retry with correct extension), proceeding without cache"
                );
            }
        }
    }

    // Record cache miss
    if !nocache {
        state.stats.record_cache_miss();
    }

    // Time the resize operation
    let resize_start = std::time::Instant::now();

    // Check image dimensions before full decode (prevents image bomb attacks)
    let max_source_pixels = state.max_source_pixels();
    let (orig_width, orig_height) =
        match crate::resize::check_image_dimensions(&image_data, max_source_pixels) {
            Ok(dims) => dims,
            Err(e) => {
                tracing::error!(
                    source_url = %full_url,
                    max_source_pixels = max_source_pixels,
                    error = %e,
                    "Image dimensions check failed"
                );
                return Err(fail_inflight(&mut flight_leader, e));
            }
        };
    tracing::debug!(
        "Source dimensions: {}x{} ({} pixels)",
        orig_width,
        orig_height,
        orig_width as u64 * orig_height as u64
    );
    if orig_width == 0 || orig_height == 0 {
        return Err(fail_inflight(
            &mut flight_leader,
            AppError::DecodeError("source image has a zero dimension".to_string()),
        ));
    }
    let (width, height) = crate::resize::bound_output_dimensions(
        orig_width as u32,
        orig_height as u32,
        width,
        height,
        fit,
        max_width,
        max_height,
    );

    // Decode, resize and encode on the blocking pool. The permit covers only that work.
    let permit = state
        .resize_permits
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| {
            fail_inflight(
                &mut flight_leader,
                AppError::EncodeError("internal".to_string()),
            )
        })?;
    let pixels = image_data;
    let encode_as = final_output_extension.to_string();
    let logged_url = full_url.clone();
    let logged_type = content_type.clone();
    let output_data = match tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let (img, extent) = decode_image_exact(&pixels, width, height).map_err(|e| {
            tracing::error!(
                source_url = %logged_url,
                content_type = ?logged_type,
                data_size = pixels.len(),
                error = %e,
                "Failed to decode source image"
            );
            e
        })?;
        let resized = resize_image_exact(img, extent, width, height, fit)?;
        encode_image(resized, &encode_as, quality).map_err(|e| {
            tracing::error!(
                source_url = %logged_url,
                output_extension = %encode_as,
                target_width = width,
                target_height = height,
                quality = quality,
                error = %e,
                "Failed to encode resized image"
            );
            e
        })
    })
    .await
    {
        Ok(result) => noted(&mut flight_leader, result)?,
        Err(_) => {
            return Err(fail_inflight(
                &mut flight_leader,
                AppError::EncodeError("internal".to_string()),
            ));
        }
    };

    let resize_time = resize_start.elapsed();

    // Record resize timing stats for this subdomain/template
    let subdomain_key = subdomain.as_deref().unwrap_or("_default");
    let template_key = template_name_used.as_deref().unwrap_or("_direct");
    state.resize_stats.record_timing(
        subdomain_key,
        template_key,
        resize_time.as_secs_f64() * 1000.0,
    );

    // Lazy GC of stale resize stats
    state.resize_stats.maybe_gc();

    // Store in resize cache (fire and forget, don't block response)
    let subdomain_for_cache = subdomain.clone();
    let cache_clone = DiskCache::new(resize_cache_path, subdomain_for_cache.as_deref());
    let cache_key_clone = final_cache_key.clone();
    let extension_clone = final_output_extension.to_string();
    let shared = Arc::new(crate::inflight::SharedImage {
        data: output_data.clone(),
        output_extension: final_output_extension,
        original_extension,
    });
    if let Some(leader) = flight_leader.as_ref() {
        leader.publish(Arc::clone(&shared));
    }
    // The key stays claimed until the result is on disk, so a request that
    // arrives in between is served from memory instead of resizing again.
    let leader = flight_leader.take();
    tokio::spawn(async move {
        if let Err(e) = cache_clone
            .put(&cache_key_clone, &extension_clone, &shared.data)
            .await
        {
            tracing::error!("Failed to cache image: {}", e);
        }
        drop(leader);
    });

    let total_time = request_start.elapsed();
    let metrics = ImageMetrics::processed(total_time, fetch_time, resize_time);
    if nocache {
        tracing::info!(
            "Processed (nocache): {} ({} bytes) - fetched in {}, resized in {}, replied in {}",
            final_cache_key,
            output_data.len(),
            format_duration(fetch_time),
            format_duration(resize_time),
            format_duration(total_time)
        );
    } else {
        tracing::info!(
            "Processed: {} ({} bytes) - fetched in {}, resized in {}, replied in {}",
            final_cache_key,
            output_data.len(),
            format_duration(fetch_time),
            format_duration(resize_time),
            format_duration(total_time)
        );
    }
    image_response(
        output_data,
        final_output_extension,
        original_extension,
        quality,
        metrics,
        nocache,
    )
}

/// Timing metrics for image processing
struct ImageMetrics {
    total_ms: f64,
    fetch_ms: Option<f64>,
    resize_ms: Option<f64>,
    cache_hit: bool,
}

impl ImageMetrics {
    fn cache_hit(total: std::time::Duration) -> Self {
        Self {
            total_ms: total.as_secs_f64() * 1000.0,
            fetch_ms: None,
            resize_ms: None,
            cache_hit: true,
        }
    }

    /// Served from another request that was already in flight.
    fn coalesced(total: std::time::Duration) -> Self {
        Self {
            total_ms: total.as_secs_f64() * 1000.0,
            fetch_ms: None,
            resize_ms: None,
            cache_hit: false,
        }
    }

    fn processed(
        total: std::time::Duration,
        fetch: std::time::Duration,
        resize: std::time::Duration,
    ) -> Self {
        Self {
            total_ms: total.as_secs_f64() * 1000.0,
            fetch_ms: Some(fetch.as_secs_f64() * 1000.0),
            resize_ms: Some(resize.as_secs_f64() * 1000.0),
            cache_hit: false,
        }
    }
}

/// How many cancelled leaders a request will replace before it does the work
/// without registering. Eight disconnects in a row is already a client problem;
/// past that, waiting longer would only pile the same work up.
const INFLIGHT_TAKEOVERS: usize = 8;

fn fail_inflight(leader: &mut Option<crate::inflight::Leader>, err: AppError) -> AppError {
    if let Some(leader) = leader.take() {
        leader.fail(Arc::new(err.clone()));
    }
    err
}

fn noted<T>(leader: &mut Option<crate::inflight::Leader>, result: Result<T>) -> Result<T> {
    result.map_err(|err| fail_inflight(leader, err))
}

fn coalesced_response(
    shared: Arc<crate::inflight::SharedImage>,
    quality: u8,
    total: std::time::Duration,
) -> Result<Response> {
    let mut response = image_response(
        shared.data.clone(),
        shared.output_extension,
        shared.original_extension,
        quality,
        ImageMetrics::coalesced(total),
        false,
    )?;
    response.headers_mut().insert(
        "X-Speedwholes-Coalesced",
        axum::http::HeaderValue::from_static("1"),
    );
    Ok(response)
}

/// Build an image response with proper headers and timing metrics.
/// When `nocache` is true, sets no-cache headers and marks cache as "bypass".
fn image_response(
    data: Vec<u8>,
    output_extension: &str,
    original_extension: &str,
    quality: u8,
    metrics: ImageMetrics,
    nocache: bool,
) -> Result<Response> {
    let mime = mime_for_extension(output_extension);

    let (cache_control, pragma, cache_status) = if nocache {
        (
            "no-cache, no-store, must-revalidate",
            Some("no-cache"),
            "bypass",
        )
    } else {
        (
            "public, max-age=31536000",
            None,
            if metrics.cache_hit { "hit" } else { "miss" },
        )
    };

    let mut response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime)
        .header(header::CACHE_CONTROL, cache_control)
        .header("X-Speedwholes-Total-Ms", format!("{:.2}", metrics.total_ms))
        .header("X-Speedwholes-Cache", cache_status)
        .header("X-Speedwholes-Original-Format", original_extension)
        .header("X-Speedwholes-Output-Format", output_extension)
        .header("X-Speedwholes-Quality", quality.to_string());

    if let Some(pragma_val) = pragma {
        response = response.header(header::PRAGMA, pragma_val);
    }
    if let Some(fetch_ms) = metrics.fetch_ms {
        response = response.header("X-Speedwholes-Fetch-Ms", format!("{:.2}", fetch_ms));
    }
    if let Some(resize_ms) = metrics.resize_ms {
        response = response.header("X-Speedwholes-Resize-Ms", format!("{:.2}", resize_ms));
    }

    response
        .body(axum::body::Body::from(data))
        .map_err(|_| AppError::EncodeError("failed to build image response".to_string()))
}

/// Sniff a supported image. JPEG is reported as `jpg` so it matches the MIME map.
fn sniff_image_format(data: &[u8]) -> Option<&'static str> {
    match image::guess_format(data).ok()? {
        image::ImageFormat::Jpeg => Some("jpg"),
        image::ImageFormat::Png => Some("png"),
        image::ImageFormat::Gif => Some("gif"),
        image::ImageFormat::WebP => Some("webp"),
        image::ImageFormat::Bmp => Some("bmp"),
        _ => None,
    }
}

fn require_image(data: &[u8]) -> Result<&'static str> {
    sniff_image_format(data)
        .ok_or_else(|| AppError::DecodeError("source is not a supported image".to_string()))
}

/// Build a bypass response - serves source image directly without resize.
/// Sets appropriate headers indicating bypass mode.
fn bypass_image_response(
    data: Vec<u8>,
    extension: &str,
    total_time: std::time::Duration,
    fetch_time: std::time::Duration,
    source_cache_hit: bool,
    nocache: bool,
) -> Result<Response> {
    let mime = mime_for_extension(extension);
    let total_ms = total_time.as_secs_f64() * 1000.0;
    let fetch_ms = fetch_time.as_secs_f64() * 1000.0;

    let (cache_control, pragma) = if nocache {
        ("no-cache, no-store, must-revalidate", Some("no-cache"))
    } else {
        ("public, max-age=31536000", None)
    };

    let source_status = if source_cache_hit { "hit" } else { "miss" };

    let mut response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime)
        .header(header::CACHE_CONTROL, cache_control)
        .header("X-Speedwholes-Mode", "bypass")
        .header("X-Speedwholes-Total-Ms", format!("{:.2}", total_ms))
        .header("X-Speedwholes-Fetch-Ms", format!("{:.2}", fetch_ms))
        .header("X-Speedwholes-Source-Cache", source_status)
        .header("X-Speedwholes-Format", extension);

    if let Some(pragma_val) = pragma {
        response = response.header(header::PRAGMA, pragma_val);
    }

    response
        .body(axum::body::Body::from(data))
        .map_err(|_| AppError::EncodeError("failed to build image response".to_string()))
}

/// SSE endpoint for streaming stats every 5 seconds
pub async fn stats_sse(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = std::result::Result<Event, Infallible>>> {
    // Clone state for the initial event
    let state_for_initial = state.clone();

    // Send an initial event immediately
    let initial = stream::once(async move {
        let snapshot = state_for_initial.stats.snapshot();
        let json = serde_json::to_string(&snapshot).unwrap_or_default();
        Ok(Event::default().data(json))
    });

    // Stream that sends updates every 5 seconds
    let stream = stream::unfold(state, |state| async move {
        // Wait 5 seconds between updates
        tokio::time::sleep(Duration::from_secs(5)).await;

        let snapshot = state.stats.snapshot();
        let json = serde_json::to_string(&snapshot).unwrap_or_default();
        let event = Event::default().data(json);

        Some((Ok(event), state))
    });

    Sse::new(initial.chain(stream)).keep_alive(KeepAlive::default())
}

/// GET /config007/stats - returns current stats as JSON (one-shot)
pub async fn stats_json(State(state): State<AppState>) -> Json<StatsSnapshot> {
    Json(state.stats.snapshot())
}

/// GET /config007/disk-stats - returns disk usage per subdomain in KB
pub async fn disk_stats(State(state): State<AppState>) -> Json<DiskStatsResponse> {
    let cache_path = state.get_cache_path();
    let stats = get_disk_stats(&cache_path).await;
    Json(stats)
}

/// GET /config007/subdomains - returns all subdomain configs
pub async fn subdomains_get(State(state): State<AppState>) -> Json<Vec<SubdomainConfig>> {
    let configs = state.subdomain_configs.read().await.clone();
    Json(configs)
}

/// POST /config007/subdomains - updates all subdomain configs (deprecated, use per-subdomain endpoints)
pub async fn subdomains_post(
    State(state): State<AppState>,
    Json(new_configs): Json<Vec<SubdomainConfig>>,
) -> impl IntoResponse {
    // Track which subdomains need signature cache invalidation
    let subdomains_to_invalidate: Vec<String>;
    let subdomains_to_delete: Vec<String>;

    {
        let old_configs = state.subdomain_configs.read().await;

        // Find subdomains whose signing_key changed
        subdomains_to_invalidate = new_configs
            .iter()
            .filter_map(|new| {
                let old = old_configs.iter().find(|o| o.name == new.name);
                match old {
                    Some(old_config) if old_config.signing_key != new.signing_key => {
                        Some(new.name.clone())
                    }
                    None if new.signing_key.is_some() => {
                        // New subdomain with signing - no cache to invalidate but log it
                        None
                    }
                    _ => None,
                }
            })
            .collect();

        // Find subdomains that were removed
        subdomains_to_delete = old_configs
            .iter()
            .filter_map(|old| {
                if !new_configs.iter().any(|n| n.name == old.name) {
                    Some(old.name.clone())
                } else {
                    None
                }
            })
            .collect();

        // Invalidate cache for removed subdomains that had signing keys
        for old in old_configs.iter() {
            if old.signing_key.is_some() && !new_configs.iter().any(|n| n.name == old.name) {
                state.signature_cache.invalidate_subdomain(&old.name);
            }
        }
    }

    // Invalidate signature caches for changed subdomains
    for subdomain in &subdomains_to_invalidate {
        state.signature_cache.invalidate_subdomain(subdomain);
    }

    // Delete files for removed subdomains
    for subdomain_name in &subdomains_to_delete {
        if let Err(e) = state.delete_subdomain_file(subdomain_name).await {
            tracing::warn!(
                "Failed to delete subdomain config file for '{}': {}",
                subdomain_name,
                e
            );
        }
    }

    for config in &new_configs {
        warn_if_signing_misconfigured(config);
    }
    {
        let mut configs = state.subdomain_configs.write().await;
        *configs = new_configs;
        tracing::info!("Subdomain configs updated: {} entries", configs.len());
    }

    // Persist all subdomain configs to individual files
    if let Err(e) = state.persist_all_subdomains().await {
        tracing::error!("Failed to persist subdomain configs: {}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(state.subdomain_configs.read().await.clone()),
        );
    }

    (
        StatusCode::OK,
        Json(state.subdomain_configs.read().await.clone()),
    )
}

/// POST /config007/subdomain/:name - save a single subdomain config
pub async fn subdomain_save(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
    Json(new_config): Json<SubdomainConfig>,
) -> impl IntoResponse {
    // Validate that the name in the path matches the config
    if name != new_config.name {
        return json_error(StatusCode::BAD_REQUEST, "Subdomain name mismatch");
    }
    warn_if_signing_misconfigured(&new_config);

    // Track if we need to invalidate signature cache
    let should_invalidate;
    let is_new;

    {
        let old_configs = state.subdomain_configs.read().await;
        let old_config = old_configs.iter().find(|c| c.name == name);

        is_new = old_config.is_none();
        should_invalidate =
            matches!(old_config, Some(old) if old.signing_key != new_config.signing_key);
    }

    // Invalidate signature cache if signing key changed
    if should_invalidate {
        state.signature_cache.invalidate_subdomain(&name);
        tracing::info!("Invalidated signature cache for subdomain '{}'", name);
    }

    // Update in-memory config
    {
        let mut configs = state.subdomain_configs.write().await;
        if let Some(pos) = configs.iter().position(|c| c.name == name) {
            configs[pos] = new_config.clone();
            tracing::info!("Updated subdomain config '{}'", name);
        } else {
            configs.push(new_config.clone());
            tracing::info!("Added new subdomain config '{}'", name);
        }
    }

    // Persist to individual file
    if let Err(e) = state.persist_subdomain(&name).await {
        tracing::error!("Failed to persist subdomain config '{}': {}", name, e);
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to save: {}", e),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "success": true,
            "message": if is_new { "Subdomain created" } else { "Subdomain updated" }
        })),
    )
        .into_response()
}

/// DELETE /config007/subdomain/:name - delete a subdomain config
pub async fn subdomain_delete(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> impl IntoResponse {
    // Check if subdomain exists and has signing key
    let had_signing_key;
    {
        let configs = state.subdomain_configs.read().await;
        if let Some(config) = configs.iter().find(|c| c.name == name) {
            had_signing_key = config.signing_key.is_some();
        } else {
            return json_error(StatusCode::NOT_FOUND, "Subdomain not found");
        }
    }

    // Invalidate signature cache if it had a signing key
    if had_signing_key {
        state.signature_cache.invalidate_subdomain(&name);
    }

    // Remove from in-memory config
    {
        let mut configs = state.subdomain_configs.write().await;
        configs.retain(|c| c.name != name);
        tracing::info!("Removed subdomain config '{}'", name);
    }

    // Delete the file
    if let Err(e) = state.delete_subdomain_file(&name).await {
        tracing::error!("Failed to delete subdomain config file '{}': {}", name, e);
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to delete: {}", e),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "success": true,
            "message": "Subdomain deleted"
        })),
    )
        .into_response()
}

/// Request body for partial subdomain params update (PATCH)
/// All fields are optional - only provided fields will be updated
#[derive(serde::Deserialize)]
pub struct SubdomainParamsUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_template: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signing_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signing_required: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_sources: Option<Vec<String>>,
    /// Set to true to clear the signing_key (since None means "don't change")
    #[serde(default)]
    pub clear_signing_key: bool,
    /// Set to true to clear the source_template
    #[serde(default)]
    pub clear_source_template: bool,
    /// Set to true to clear allowed_sources (use global)
    #[serde(default)]
    pub clear_allowed_sources: bool,
    /// Name of the default template (set to change, None means don't change)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_template: Option<String>,
    /// Set to true to clear the default_template
    #[serde(default)]
    pub clear_default_template: bool,
}

/// PATCH /config007/subdomain/:name/params - partially update subdomain params
pub async fn subdomain_params_patch(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
    Json(updates): Json<SubdomainParamsUpdate>,
) -> impl IntoResponse {
    // Check if subdomain exists
    let should_invalidate_cache;
    {
        let configs = state.subdomain_configs.read().await;
        let config = configs.iter().find(|c| c.name == name);
        if config.is_none() {
            return json_error(StatusCode::NOT_FOUND, "Subdomain not found");
        }

        // Check if signing key is changing
        let old_key = config.and_then(|c| c.signing_key.as_ref());
        let new_key = updates.signing_key.as_ref();
        should_invalidate_cache =
            updates.clear_signing_key || (new_key.is_some() && new_key != old_key);
    }

    // Invalidate signature cache if signing key changed
    if should_invalidate_cache {
        state.signature_cache.invalidate_subdomain(&name);
        tracing::info!(
            "Invalidated signature cache for subdomain '{}' (key changed)",
            name
        );
    }

    // Apply updates to in-memory config
    {
        let mut configs = state.subdomain_configs.write().await;
        if let Some(config) = configs.iter_mut().find(|c| c.name == name) {
            if let Some(max_width) = updates.max_width {
                config.max_width = Some(max_width);
            }
            if let Some(max_height) = updates.max_height {
                config.max_height = Some(max_height);
            }
            if updates.clear_source_template {
                config.source_template = None;
            } else if let Some(source_template) = updates.source_template {
                config.source_template = Some(source_template);
            }
            if updates.clear_signing_key {
                config.signing_key = None;
            } else if let Some(signing_key) = updates.signing_key {
                config.signing_key = Some(signing_key);
            }
            if let Some(signing_required) = updates.signing_required {
                config.signing_required = signing_required;
            }
            if updates.clear_allowed_sources {
                config.allowed_sources = None;
            } else if let Some(allowed_sources) = updates.allowed_sources {
                config.allowed_sources = Some(allowed_sources);
            }
            if updates.clear_default_template {
                config.default_template = None;
            } else if let Some(default_template) = updates.default_template {
                config.default_template = Some(default_template);
            }

            tracing::info!("Updated subdomain params for '{}'", name);
        }
    }

    // Persist to file
    if let Err(e) = state.persist_subdomain(&name).await {
        tracing::error!("Failed to persist subdomain config '{}': {}", name, e);
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to save: {}", e),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "success": true,
            "message": "Subdomain params updated"
        })),
    )
        .into_response()
}

/// PUT /config007/subdomain/:name/template/:template_name - create or update a single template
pub async fn subdomain_template_put(
    State(state): State<AppState>,
    axum::extract::Path((subdomain_name, template_name)): axum::extract::Path<(String, String)>,
    Json(template): Json<crate::config::ResizeTemplate>,
) -> impl IntoResponse {
    // Validate that template name in path matches body
    if template_name != template.name {
        return json_error(
            StatusCode::BAD_REQUEST,
            "Template name in path must match body",
        );
    }

    // Check if subdomain exists and update template
    let is_new;
    {
        let mut configs = state.subdomain_configs.write().await;
        let config = configs.iter_mut().find(|c| c.name == subdomain_name);

        match config {
            None => {
                return json_error(StatusCode::NOT_FOUND, "Subdomain not found");
            }
            Some(config) => {
                // Initialize templates vec if needed
                if config.templates.is_none() {
                    config.templates = Some(Vec::new());
                }

                let templates = config.templates.as_mut().unwrap();

                // Find existing template or add new one
                if let Some(pos) = templates.iter().position(|t| t.name == template_name) {
                    templates[pos] = template;
                    is_new = false;
                    tracing::info!(
                        "Updated template '{}' in subdomain '{}'",
                        template_name,
                        subdomain_name
                    );
                } else {
                    templates.push(template);
                    is_new = true;
                    tracing::info!(
                        "Added template '{}' to subdomain '{}'",
                        template_name,
                        subdomain_name
                    );
                }
            }
        }
    }

    // Persist to file
    if let Err(e) = state.persist_subdomain(&subdomain_name).await {
        tracing::error!(
            "Failed to persist subdomain config '{}': {}",
            subdomain_name,
            e
        );
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to save: {}", e),
        );
    }

    (
        if is_new {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        Json(serde_json::json!({
            "success": true,
            "message": if is_new { "Template created" } else { "Template updated" }
        })),
    )
        .into_response()
}

/// DELETE /config007/subdomain/:name/template/:template_name - delete a single template
pub async fn subdomain_template_delete(
    State(state): State<AppState>,
    axum::extract::Path((subdomain_name, template_name)): axum::extract::Path<(String, String)>,
) -> impl IntoResponse {
    // Check if subdomain and template exist
    {
        let mut configs = state.subdomain_configs.write().await;
        let config = configs.iter_mut().find(|c| c.name == subdomain_name);

        match config {
            None => {
                return json_error(StatusCode::NOT_FOUND, "Subdomain not found");
            }
            Some(config) => {
                match &mut config.templates {
                    None => {
                        return json_error(StatusCode::NOT_FOUND, "Template not found");
                    }
                    Some(templates) => {
                        let original_len = templates.len();
                        templates.retain(|t| t.name != template_name);

                        if templates.len() == original_len {
                            return json_error(StatusCode::NOT_FOUND, "Template not found");
                        }

                        // If templates is now empty, set to None for cleaner JSON
                        if templates.is_empty() {
                            config.templates = None;
                        }

                        tracing::info!(
                            "Deleted template '{}' from subdomain '{}'",
                            template_name,
                            subdomain_name
                        );
                    }
                }
            }
        }
    }

    // Persist to file
    if let Err(e) = state.persist_subdomain(&subdomain_name).await {
        tracing::error!(
            "Failed to persist subdomain config '{}': {}",
            subdomain_name,
            e
        );
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to save: {}", e),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "success": true,
            "message": "Template deleted"
        })),
    )
        .into_response()
}

/// Request body for signing a URL
#[derive(serde::Deserialize)]
pub struct SignRequest {
    /// The subdomain to use for signing (must have a signing_key configured)
    pub subdomain: String,
    /// The path to sign (e.g., "/w/300/h/200/https://example.com/image.jpg")
    /// Should NOT include /resize/ prefix or signature
    pub path: String,
}

/// Response from signing a URL
#[derive(serde::Serialize)]
pub struct SignResponse {
    /// The generated signature
    pub signature: String,
    /// The canonical path that was signed (for debugging)
    pub signing_path: String,
    /// The full signed URL ready to use
    pub signed_url: String,
}

/// POST /config007/sign - Generate a signed URL for a subdomain
pub async fn sign_url(
    State(state): State<AppState>,
    Json(req): Json<SignRequest>,
) -> Result<Json<SignResponse>> {
    // Find the subdomain config
    let subdomain_configs = state.subdomain_configs.read().await;
    let sub_config = subdomain_configs
        .iter()
        .find(|c| c.name == req.subdomain)
        .ok_or_else(|| {
            AppError::InvalidFitMode(format!("Subdomain '{}' not found", req.subdomain))
        })?;

    // Get the signing key
    let signing_key = sub_config
        .signing_key
        .as_ref()
        .ok_or(AppError::SigningNotConfigured)?;

    // Parse the path to extract params and URL for canonical path building
    // The path should be like "/w/300/h/200/https://example.com/image.jpg"
    let path = req.path.trim_start_matches('/');

    // Find where the source URL starts
    let url_start = path
        .find("http://")
        .or_else(|| path.find("https://"))
        .ok_or_else(|| AppError::InvalidFitMode("No source URL found in path".to_string()))?;

    let params_section = &path[..url_start];
    let source_url = &path[url_start..];

    // Parse params
    let parts: Vec<&str> = params_section
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();

    if !parts.len().is_multiple_of(2) {
        return Err(AppError::InvalidFitMode(
            "Parameters must be in key/value pairs".to_string(),
        ));
    }

    // Build params list (excluding any existing 's' param)
    let params: Vec<(&str, &str)> = parts
        .chunks(2)
        .filter(|chunk| chunk[0] != "s")
        .map(|chunk| (chunk[0], chunk[1]))
        .collect();

    // Build canonical signing path
    let signing_path = crate::signing::build_canonical_path(params.iter().copied(), source_url);

    // Generate signature
    let signature = crate::signing::generate_signature(signing_key, &signing_path);

    // Build signed URL - insert signature into path
    let signed_url = format!(
        "/resize/{}/s/{}/{}",
        params_section.trim_end_matches('/'),
        signature,
        source_url
    );

    Ok(Json(SignResponse {
        signature,
        signing_path,
        signed_url,
    }))
}

// ============================================================================
// Logs API
// ============================================================================

/// Query parameters for logs endpoint
#[derive(Debug, Deserialize)]
pub struct LogsQuery {
    /// Number of lines to return (default 1000)
    #[serde(default = "default_lines")]
    lines: usize,
    /// Offset from the end of the file (for pagination, default 0)
    #[serde(default)]
    offset: usize,
}

fn default_lines() -> usize {
    1000
}

/// Response for logs endpoint
#[derive(Debug, serde::Serialize)]
pub struct LogsResponse {
    /// Array of log entries (raw JSON strings, parsed by frontend)
    pub entries: Vec<serde_json::Value>,
    /// Total lines returned
    pub count: usize,
    /// Whether there are more logs available (for pagination)
    pub has_more: bool,
    /// Current offset
    pub offset: usize,
}

/// GET /config007/logs - returns recent log entries from systemd journal
pub async fn logs_get(axum::extract::Query(query): axum::extract::Query<LogsQuery>) -> Response {
    use std::process::Command;

    // Calculate how many entries to fetch (lines + offset to support pagination)
    let fetch_count = query.lines + query.offset;

    // Run journalctl to get logs from our service in JSON format
    let output = Command::new("journalctl")
        .args([
            "-u",
            "speedwholes",
            "-o",
            "json",
            "--no-pager",
            "-n",
            &fetch_count.to_string(),
            "-r", // Reverse order (newest first)
        ])
        .output();

    match output {
        Ok(out) => {
            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr);
                tracing::error!(stderr = %stderr, "journalctl command failed");

                // Check for permission error and provide helpful message
                let error_msg = if stderr.contains("insufficient permissions")
                    || stderr.contains("No journal files were opened")
                {
                    "Permission denied reading journal. Fix: sudo usermod -a -G systemd-journal speedwholes && sudo systemctl restart speedwholes".to_string()
                } else {
                    format!("Failed to read journal: {}", stderr)
                };

                return json_error(StatusCode::INTERNAL_SERVER_ERROR, error_msg);
            }

            // Parse each line as JSON (journalctl -o json outputs one JSON object per line)
            let all_entries: Vec<serde_json::Value> = String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter_map(|line| {
                    serde_json::from_str::<serde_json::Value>(line)
                        .ok()
                        .map(|journal_entry| {
                            // Extract the MESSAGE field which contains our tracing JSON
                            if let Some(msg) = journal_entry.get("MESSAGE").and_then(|m| m.as_str())
                            {
                                // Try to parse MESSAGE as JSON (our structured logs)
                                serde_json::from_str(msg).unwrap_or_else(|_| {
                                    // If not JSON, wrap the plain message
                                    serde_json::json!({
                                        "message": msg,
                                        "timestamp": journal_entry.get("__REALTIME_TIMESTAMP")
                                    })
                                })
                            } else {
                                // Fallback: return the raw journal entry
                                journal_entry
                            }
                        })
                })
                .collect();

            // Apply offset for pagination (skip first N entries since we got newest first)
            let entries: Vec<serde_json::Value> = all_entries
                .into_iter()
                .skip(query.offset)
                .take(query.lines)
                .collect();

            let count = entries.len();
            // has_more is true if we got more entries than offset + lines requested
            let has_more = fetch_count > query.offset + count;

            Json(LogsResponse {
                entries,
                count,
                has_more,
                offset: query.offset,
            })
            .into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to execute journalctl");
            json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to execute journalctl: {}", e),
            )
        }
    }
}

/// GET /config007/resize-stats - Get resize timing statistics
pub async fn resize_stats_get(State(state): State<AppState>) -> impl IntoResponse {
    let snapshot = state.resize_stats.get_snapshot();
    Json(snapshot)
}

/// DELETE /config007/resize-stats - Reset all resize timing statistics
pub async fn resize_stats_reset(State(state): State<AppState>) -> impl IntoResponse {
    state.resize_stats.reset_all();
    Json(serde_json::json!({ "success": true, "message": "All resize stats reset" }))
}

/// DELETE /config007/resize-stats/:subdomain - Reset resize stats for a specific subdomain
pub async fn resize_stats_reset_subdomain(
    State(state): State<AppState>,
    axum::extract::Path(subdomain): axum::extract::Path<String>,
) -> impl IntoResponse {
    let removed = state.resize_stats.reset_subdomain(&subdomain);
    if removed {
        Json(serde_json::json!({
            "success": true,
            "message": format!("Resize stats reset for subdomain '{}'", subdomain)
        }))
    } else {
        Json(serde_json::json!({
            "success": true,
            "message": format!("No stats found for subdomain '{}'", subdomain)
        }))
    }
}

/// DELETE /config007/subdomain/:name/cache/resize - Clear resize cache for a subdomain
pub async fn subdomain_cache_clear_resize(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Response {
    match clear_subdomain_cache(&state.cache_path, "resize", &name).await {
        Ok(result) => Json(serde_json::json!({
            "success": true,
            "subdomain": name,
            "cache_type": "resize",
            "files_deleted": result.files_deleted,
            "bytes_freed": result.bytes_freed,
        }))
        .into_response(),
        Err(e) => {
            tracing::error!(subdomain = %name, error = %e, "Failed to clear resize cache");
            json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to clear resize cache: {}", e),
            )
        }
    }
}

/// DELETE /config007/subdomain/:name/cache/source - Clear source cache for a subdomain
pub async fn subdomain_cache_clear_source(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Response {
    match clear_subdomain_cache(&state.cache_path, "source", &name).await {
        Ok(result) => Json(serde_json::json!({
            "success": true,
            "subdomain": name,
            "cache_type": "source",
            "files_deleted": result.files_deleted,
            "bytes_freed": result.bytes_freed,
        }))
        .into_response(),
        Err(e) => {
            tracing::error!(subdomain = %name, error = %e, "Failed to clear source cache");
            json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to clear source cache: {}", e),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tenant_list_overrides_allow_all() {
        let tenant = vec!["cdn.example.com".to_string()];
        let policy = resolve_source_policy(true, &[], Some(&tenant));
        assert!(!policy.allow_all);
        assert_eq!(policy.patterns, tenant);
    }

    #[test]
    fn empty_tenant_list_falls_back_to_global() {
        let global = vec!["127.0.0.1".to_string()];
        let policy = resolve_source_policy(false, &global, Some(&[]));
        assert!(!policy.allow_all);
        assert_eq!(policy.patterns, global);
    }

    #[test]
    fn missing_tenant_list_uses_allow_all() {
        let policy = resolve_source_policy(true, &["example.com".to_string()], None);
        assert!(policy.allow_all);
        assert!(policy.patterns.is_empty());
    }

    #[test]
    fn global_empty_list_denies() {
        let policy = resolve_source_policy(false, &[], None);
        assert!(!policy.allow_all);
        assert!(policy.patterns.is_empty());
    }

    #[test]
    fn interpolate_rejects_dot_segments_and_at() {
        assert!(interpolate_template(
            "https://cdn.example.com/{0}/{1}",
            &["..".into(), "x".into()]
        )
        .is_err());
        assert!(
            interpolate_template("https://cdn.example.com/{0}", &["@evil.com".into()]).is_err()
        );
        let ok = interpolate_template(
            "https://cdn.example.com/o/{0}?alt=media",
            &["cimg%2Fa%2Fb.jpg".into()],
        )
        .unwrap();
        assert_eq!(ok, "https://cdn.example.com/o/cimg%2Fa%2Fb.jpg?alt=media");
    }
}
