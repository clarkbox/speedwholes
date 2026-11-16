#![allow(clippy::field_reassign_with_default)]
//! End-to-end resize behaviour against an in-process upstream.
//!
//! These tests set `allow_all_sources: true` explicitly so they do not depend
//! on the default of that flag.

use axum::http::StatusCode;

use crate::config::{ResizeTemplate, ServerConfig, SubdomainConfig};
use crate::test_support::{self, jpeg_bytes};

fn allow_all() -> ServerConfig {
    let mut cfg = ServerConfig::default();
    cfg.allow_all_sources = true;
    cfg.allow_private_networks = true;
    // These tests address the apex (`example.com`), which the default policy rejects.
    cfg.unknown_host_policy = "global".to_string();
    cfg.timeout = 10;
    cfg
}

fn cb_tenant(port: u16) -> SubdomainConfig {
    SubdomainConfig {
        name: "cb".to_string(),
        max_width: None,
        max_height: None,
        source_template: Some(format!("http://127.0.0.1:{port}/o/{{0}}?alt=media")),
        signing_key: None,
        signing_required: false,
        allowed_sources: None,
        allow_bypass: None,
        templates: Some(vec![ResizeTemplate {
            name: "1000m".to_string(),
            width: Some(1000),
            height: Some(1000),
            fit: Some("contain".to_string()),
            quality: Some(78),
            format: Some("jpg".to_string()),
            source_template: None,
        }]),
        default_template: None,
    }
}

#[tokio::test]
async fn template_parts_double_slash_keeps_encoded_slash() {
    let (addr, counter) = test_support::spawn_upstream().await;
    let (app, _dir) = test_support::test_app(allow_all(), vec![cb_tenant(addr.port())]);

    let (status, headers, body) = test_support::get(
        app,
        "cb.example.com",
        "/resize//t/1000m/p/cimg%2Fgal%2Fabc.jpg?cver=3",
        &[],
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(
        headers.get("content-type").and_then(|v| v.to_str().ok()),
        Some("image/jpeg")
    );
    assert!(
        counter
            .seen_paths()
            .iter()
            .any(|p| p == "/o/cimg%2Fgal%2Fabc.jpg"),
        "upstream paths: {:?}",
        counter.seen_paths()
    );
}

#[tokio::test]
async fn bypass_returns_the_original_bytes() {
    let (addr, _counter) = test_support::spawn_upstream().await;
    let (app, _dir) = test_support::test_app(allow_all(), vec![cb_tenant(addr.port())]);

    let (status, _headers, body) = test_support::get(
        app,
        "cb.example.com",
        "/resize//b/1/p/cimg%2Fgal%2Fabc.jpg",
        &[],
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(body.as_ref(), jpeg_bytes(32, 24).as_slice());
}

#[tokio::test]
async fn explicit_dims_url_mode() {
    let (addr, _counter) = test_support::spawn_upstream().await;
    let (app, _dir) = test_support::test_app(allow_all(), vec![]);
    let path = format!(
        "/resize/w/100/h/100/u/http://127.0.0.1:{}/img.png",
        addr.port()
    );

    let (status, _headers, body) = test_support::get(app, "example.com", &path, &[]).await;

    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let img = image::load_from_memory(&body).expect("decode output");
    assert_eq!((img.width(), img.height()), (100, 100));
}

fn tenant(name: &str, allowed: Option<Vec<String>>) -> SubdomainConfig {
    SubdomainConfig {
        name: name.to_string(),
        max_width: None,
        max_height: None,
        source_template: None,
        signing_key: None,
        signing_required: false,
        allowed_sources: allowed,
        allow_bypass: None,
        templates: None,
        default_template: None,
    }
}

#[tokio::test]
async fn tenant_allowlist_enforced_even_when_allow_all() {
    let (addr, _counter) = test_support::spawn_upstream().await;
    let mut cfg = allow_all();
    cfg.allow_all_sources = true;
    let (app, _dir) = test_support::test_app(
        cfg,
        vec![tenant("cdn", Some(vec!["cdn.example.com".to_string()]))],
    );
    let path = format!(
        "/resize/w/10/h/10/u/http://127.0.0.1:{}/img.png",
        addr.port()
    );
    let (status, _headers, body) = test_support::get(app, "cdn.example.com", &path, &[]).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "{}",
        String::from_utf8_lossy(&body)
    );
    assert!(body
        .windows(b"SOURCE_NOT_ALLOWED".len())
        .any(|w| w == b"SOURCE_NOT_ALLOWED"));
}

#[tokio::test]
async fn no_allowlist_denies_when_allow_all_is_false() {
    let (addr, _counter) = test_support::spawn_upstream().await;
    let mut cfg = ServerConfig::default();
    cfg.allow_all_sources = false;
    cfg.timeout = 10;
    let (app, _dir) = test_support::test_app(cfg, vec![tenant("open", None)]);
    let path = format!(
        "/resize/w/10/h/10/u/http://127.0.0.1:{}/img.png",
        addr.port()
    );
    let (status, _headers, body) = test_support::get(app, "open.example.com", &path, &[]).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "{}",
        String::from_utf8_lossy(&body)
    );
    assert!(String::from_utf8_lossy(&body).contains("SOURCE_NOT_ALLOWED"));
}

#[tokio::test]
async fn empty_tenant_list_falls_back_to_global() {
    let (addr, counter) = test_support::spawn_upstream().await;
    let mut cfg = ServerConfig::default();
    cfg.allow_all_sources = false;
    cfg.allow_private_networks = true;
    cfg.allowed_sources = vec!["127.0.0.1".to_string()];
    cfg.timeout = 10;
    let (app, _dir) = test_support::test_app(cfg, vec![tenant("open", Some(vec![]))]);
    let path = format!(
        "/resize/w/10/h/10/u/http://127.0.0.1:{}/img.png",
        addr.port()
    );
    let (status, _headers, body) = test_support::get(app, "open.example.com", &path, &[]).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert!(counter.hits("/img.png") >= 1);
}

fn cache_has_files(root: &std::path::Path) -> bool {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file() {
                return true;
            }
        }
    }
    false
}

#[tokio::test]
async fn bypass_rejects_non_image_and_does_not_cache_it() {
    let (addr, _counter) = test_support::spawn_upstream().await;
    let (app, dir) = test_support::test_app(allow_all(), vec![]);
    let path = format!("/resize/b/1/u/http://127.0.0.1:{}/html", addr.port());
    let (status, _headers, body) = test_support::get(app, "example.com", &path, &[]).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        String::from_utf8_lossy(&body)
    );
    assert!(String::from_utf8_lossy(&body).contains("source is not a supported image"));
    assert!(!cache_has_files(&dir.path().join("cache").join("source")));
}

#[tokio::test]
async fn bypass_content_type_follows_sniffed_bytes() {
    let (addr, _counter) = test_support::spawn_upstream().await;
    let (app, _dir) = test_support::test_app(allow_all(), vec![]);
    let path = format!("/resize/b/1/u/http://127.0.0.1:{}/png.jpg", addr.port());
    let (status, headers, body) = test_support::get(app, "example.com", &path, &[]).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(
        headers.get("content-type").and_then(|v| v.to_str().ok()),
        Some("image/png")
    );
}

#[tokio::test]
async fn responses_send_nosniff() {
    let (addr, _counter) = test_support::spawn_upstream().await;
    let (app, _dir) = test_support::test_app(allow_all(), vec![]);
    let resize = format!("/resize/w/8/h/8/u/http://127.0.0.1:{}/img.png", addr.port());
    let bypass = format!("/resize/b/1/u/http://127.0.0.1:{}/img.jpg", addr.port());
    for (path, host) in [
        (resize.as_str(), "example.com"),
        (bypass.as_str(), "example.com"),
        ("/missing", "example.com"),
        ("/health", "nobody.example.com"),
    ] {
        let (status, headers, _body) = test_support::get(app.clone(), host, path, &[]).await;
        assert!(
            status.is_success() || status == StatusCode::NOT_FOUND,
            "{path} -> {status}"
        );
        assert_eq!(
            headers
                .get("x-content-type-options")
                .and_then(|v| v.to_str().ok()),
            Some("nosniff"),
            "{path}"
        );
    }
}

#[tokio::test]
async fn bypass_can_be_disabled_per_tenant() {
    let (addr, _counter) = test_support::spawn_upstream().await;
    let mut tenant = tenant("locked", None);
    tenant.allow_bypass = Some(false);
    let (app, _dir) = test_support::test_app(allow_all(), vec![tenant]);
    let path = format!("/resize/b/1/u/http://127.0.0.1:{}/img.jpg", addr.port());
    let (status, _headers, body) = test_support::get(app, "locked.example.com", &path, &[]).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "{}",
        String::from_utf8_lossy(&body)
    );
    assert!(String::from_utf8_lossy(&body).contains("BYPASS_DISABLED"));
}

#[tokio::test]
async fn redirect_to_same_host_is_followed() {
    let (addr, _counter) = test_support::spawn_upstream().await;
    let (app, _dir) = test_support::test_app(allow_all(), vec![]);
    let path = format!(
        "/resize/w/10/h/10/u/http://127.0.0.1:{}/redirect?to=/img.png",
        addr.port()
    );
    let (status, _headers, body) = test_support::get(app, "example.com", &path, &[]).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
}

#[tokio::test]
async fn redirect_to_unlisted_host_is_rejected() {
    let (addr, counter) = test_support::spawn_upstream().await;
    let mut cfg = ServerConfig::default();
    cfg.allow_all_sources = false;
    cfg.allow_private_networks = true;
    cfg.allowed_sources = vec!["127.0.0.1".to_string()];
    cfg.unknown_host_policy = "global".to_string();
    cfg.timeout = 5;
    let (app, _dir) = test_support::test_app(cfg, vec![]);
    let path = format!(
        "/resize/w/10/h/10/u/http://127.0.0.1:{port}/redirect?to=http://localhost:{port}/img.png",
        port = addr.port()
    );
    let (status, _headers, body) = test_support::get(app, "example.com", &path, &[]).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "{}",
        String::from_utf8_lossy(&body)
    );
    assert_eq!(counter.hits("/img.png"), 0);
}

#[tokio::test]
async fn redirect_loop_is_rejected() {
    let (addr, _counter) = test_support::spawn_upstream().await;
    let (app, _dir) = test_support::test_app(allow_all(), vec![]);
    let path = format!("/resize/w/10/h/10/u/http://127.0.0.1:{}/loop", addr.port());
    let (status, _headers, body) = test_support::get(app, "example.com", &path, &[]).await;
    assert_eq!(
        status,
        StatusCode::BAD_GATEWAY,
        "{}",
        String::from_utf8_lossy(&body)
    );
    assert!(String::from_utf8_lossy(&body).contains("upstream fetch failed"));
    assert!(!String::from_utf8_lossy(&body).contains("too many redirects"));
}

#[tokio::test]
async fn localhost_allowlist_does_not_reach_upstream() {
    let (addr, counter) = test_support::spawn_upstream().await;
    let mut cfg = ServerConfig::default();
    cfg.allow_all_sources = false;
    cfg.allow_private_networks = false;
    cfg.timeout = 5;
    let (app, _dir) = test_support::test_app(
        cfg,
        vec![tenant("local", Some(vec!["localhost".to_string()]))],
    );
    let path = format!(
        "/resize/w/10/h/10/u/http://localhost:{}/img.png",
        addr.port()
    );
    let (status, _headers, body) = test_support::get(app, "local.example.com", &path, &[]).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "{}",
        String::from_utf8_lossy(&body)
    );
    assert_eq!(counter.hits("/img.png"), 0);
}

#[tokio::test]
async fn allow_all_still_blocks_metadata_address() {
    let mut cfg = ServerConfig::default();
    cfg.allow_all_sources = true;
    cfg.allow_private_networks = false;
    cfg.unknown_host_policy = "global".to_string();
    cfg.timeout = 5;
    let (app, _dir) = test_support::test_app(cfg, vec![]);
    let (status, _headers, body) = test_support::get(
        app,
        "example.com",
        "/resize/w/10/h/10/u/http://169.254.169.254/latest/meta-data/",
        &[],
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "{}",
        String::from_utf8_lossy(&body)
    );
    assert!(String::from_utf8_lossy(&body).contains("SOURCE_NOT_ALLOWED"));
}

fn signed_tenant(required: bool, key: Option<&str>) -> SubdomainConfig {
    let mut tenant = tenant("signed", None);
    tenant.signing_required = required;
    tenant.signing_key = key.map(|k| k.to_string());
    tenant
}

#[tokio::test]
async fn signing_required_rejects_missing_and_accepts_valid() {
    let (addr, _counter) = test_support::spawn_upstream().await;
    let key = "test-secret-key-at-least-32-chars!";
    let (app, _dir) = test_support::test_app(allow_all(), vec![signed_tenant(true, Some(key))]);
    let source = format!("http://127.0.0.1:{}/img.png", addr.port());
    let unsigned = format!("/resize/w/10/h/10/u/{source}");
    let (status, _headers, body) =
        test_support::get(app.clone(), "signed.example.com", &unsigned, &[]).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "{}",
        String::from_utf8_lossy(&body)
    );
    assert!(String::from_utf8_lossy(&body).contains("SIGNATURE_REQUIRED"));

    let canonical =
        crate::signing::build_canonical_path([("w", "10"), ("h", "10")].into_iter(), &source);
    let sig = crate::signing::generate_signature(key, &canonical);
    let signed = format!("/resize/w/10/h/10/s/{sig}/u/{source}");
    let (status, _headers, body) = test_support::get(app, "signed.example.com", &signed, &[]).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
}

#[tokio::test]
async fn unknown_hosts_are_rejected() {
    let (addr, _counter) = test_support::spawn_upstream().await;
    let mut cfg = allow_all();
    cfg.unknown_host_policy = "reject".to_string();
    let (app, _dir) = test_support::test_app(cfg, vec![]);
    let path = format!(
        "/resize/w/10/h/10/u/http://127.0.0.1:{}/img.png",
        addr.port()
    );
    for host in [
        "nobody.example.com",
        "example.com",
        "127.0.0.1:3000",
        "[::1]:3000",
    ] {
        let (status, _headers, body) = test_support::get(app.clone(), host, &path, &[]).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{host}: {}",
            String::from_utf8_lossy(&body)
        );
        assert!(
            String::from_utf8_lossy(&body).contains("UNKNOWN_HOST"),
            "{host}"
        );
    }
}

#[tokio::test]
async fn forwarded_host_is_ignored_unless_trusted() {
    let (addr, _counter) = test_support::spawn_upstream().await;
    let key = "test-secret-key-at-least-32-chars!";
    let tenants = vec![signed_tenant(true, Some(key)), tenant("open", None)];
    let (app, _dir) = test_support::test_app(allow_all(), tenants.clone());
    let path = format!(
        "/resize/w/10/h/10/u/http://127.0.0.1:{}/img.png",
        addr.port()
    );
    let extra = [("x-forwarded-host", "open.example.com")];
    let (status, _headers, body) =
        test_support::get(app, "signed.example.com", &path, &extra).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "{}",
        String::from_utf8_lossy(&body)
    );
    assert!(String::from_utf8_lossy(&body).contains("SIGNATURE_REQUIRED"));

    let mut cfg = allow_all();
    cfg.trust_forwarded_host = true;
    let (app, _dir) = test_support::test_app(cfg, tenants);
    let (status, _headers, body) =
        test_support::get(app, "signed.example.com", &path, &extra).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
}

#[tokio::test]
async fn unknown_host_policy_global_uses_the_allowlist() {
    let (addr, _counter) = test_support::spawn_upstream().await;
    let mut cfg = ServerConfig::default();
    cfg.unknown_host_policy = "global".to_string();
    cfg.allow_private_networks = true;
    cfg.allowed_sources = vec!["127.0.0.1".to_string()];
    cfg.timeout = 10;
    let (app, _dir) = test_support::test_app(cfg, vec![]);
    let path = format!(
        "/resize/w/10/h/10/u/http://127.0.0.1:{}/img.png",
        addr.port()
    );
    let (status, _headers, body) = test_support::get(app, "nobody.example.com", &path, &[]).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
}

#[tokio::test]
async fn signing_required_without_a_key_fails_closed() {
    let (app, _dir) = test_support::test_app(allow_all(), vec![signed_tenant(true, None)]);
    let (status, _headers, body) = test_support::get(
        app,
        "signed.example.com",
        "/resize/w/10/h/10/u/http://127.0.0.1:9/img.png",
        &[],
    )
    .await;
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "{}",
        String::from_utf8_lossy(&body)
    );
    assert!(String::from_utf8_lossy(&body).contains("SIGNING_NOT_CONFIGURED"));
}

fn limited_tenant(max_width: u32, template_width: Option<u32>) -> SubdomainConfig {
    let mut tenant = tenant("dim", None);
    tenant.max_width = Some(max_width);
    if let Some(width) = template_width {
        tenant.templates = Some(vec![ResizeTemplate {
            name: "wide".to_string(),
            width: Some(width),
            height: Some(10),
            fit: Some("contain".to_string()),
            quality: Some(80),
            format: Some("jpg".to_string()),
            source_template: None,
        }]);
    }
    tenant
}

#[tokio::test]
async fn auto_height_is_clamped_and_cover_stays_bounded() {
    let (addr, _counter) = test_support::spawn_upstream().await;
    let (app, _dir) = test_support::test_app(allow_all(), vec![]);
    let tall = format!(
        "/resize/w/4096/h/0/u/http://127.0.0.1:{}/tall.png",
        addr.port()
    );
    let (status, _headers, body) = test_support::get(app.clone(), "example.com", &tall, &[]).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let img = image::load_from_memory(&body).expect("decode tall output");
    assert!(img.height() <= 4096, "height {}", img.height());
    assert!(img.height() >= 1);

    let started = std::time::Instant::now();
    let wide = format!(
        "/resize/w/4096/h/4096/fit/cover/f/jpg/u/http://127.0.0.1:{}/wide.png",
        addr.port()
    );
    let (status, _headers, body) = test_support::get(app, "example.com", &wide, &[]).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    // Debug builds on a shared CI runner need more than a few seconds for a
    // 4096² JPEG. The assertion still fails a resize that runs away.
    let limit = if cfg!(debug_assertions) { 30 } else { 5 };
    assert!(started.elapsed() < std::time::Duration::from_secs(limit));
    let img = image::load_from_memory(&body).expect("decode wide output");
    assert_eq!((img.width(), img.height()), (4096, 4096));
}

#[tokio::test]
async fn zero_width_png_is_rejected() {
    let (addr, _counter) = test_support::spawn_upstream().await;
    let (app, _dir) = test_support::test_app(allow_all(), vec![]);
    let path = format!(
        "/resize/w/10/h/10/u/http://127.0.0.1:{}/zero.png",
        addr.port()
    );
    let (status, _headers, body) = test_support::get(app, "example.com", &path, &[]).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        String::from_utf8_lossy(&body)
    );
}

#[tokio::test]
async fn upstream_failure_body_hides_the_target() {
    let mut cfg = allow_all();
    cfg.timeout = 2;
    let (app, _dir) = test_support::test_app(cfg, vec![]);
    let (status, _headers, body) = test_support::get(
        app,
        "example.com",
        "/resize/w/8/h/8/u/http://127.0.0.1:1/x.png",
        &[],
    )
    .await;
    let text = String::from_utf8_lossy(&body);
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{text}");
    assert!(!text.contains("127.0.0.1"));
    assert!(!text.contains("error sending request"));
    assert!(text.contains("upstream fetch failed"));
}

#[tokio::test]
async fn resize_permit_does_not_block_health() {
    let (addr, _counter) = test_support::spawn_upstream().await;
    let mut cfg = allow_all();
    cfg.max_concurrent_resizes = 1;
    let (app, _dir) = test_support::test_app(cfg, vec![]);
    let path = format!(
        "/resize/w/512/h/512/fit/cover/f/jpg/nocache/1/u/http://127.0.0.1:{}/wide.png",
        addr.port()
    );
    let mut work = Vec::new();
    for _ in 0..4 {
        let app = app.clone();
        let path = path.clone();
        work.push(tokio::spawn(async move {
            test_support::get(app, "example.com", &path, &[]).await
        }));
    }
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    let health_started = std::time::Instant::now();
    let (status, _headers, body) = test_support::get(app, "example.com", "/health", &[]).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert!(health_started.elapsed() < std::time::Duration::from_millis(200));
    for job in work {
        let (status, _headers, body) = job.await.expect("resize task");
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    }
}

#[tokio::test]
async fn upstream_download_is_capped() {
    let (addr, counter) = test_support::spawn_upstream().await;
    let mut cfg = allow_all();
    cfg.max_source_bytes = 1_048_576;
    let (app, _dir) = test_support::test_app(cfg, vec![]);
    let big = format!("/resize/b/1/u/http://127.0.0.1:{}/big", addr.port());
    let (status, _headers, body) = test_support::get(app.clone(), "example.com", &big, &[]).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        String::from_utf8_lossy(&body)
    );
    assert!(String::from_utf8_lossy(&body).contains("SOURCE_TOO_LARGE"));

    let big_cl = format!("/resize/b/1/u/http://127.0.0.1:{}/big-cl", addr.port());
    let (status, _headers, body) =
        test_support::get(app.clone(), "example.com", &big_cl, &[]).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        String::from_utf8_lossy(&body)
    );
    assert!(counter.bytes_written("/big-cl") < 5 * 1024 * 1024);

    let small = format!("/resize/w/8/h/8/u/http://127.0.0.1:{}/img.png", addr.port());
    let (status, _headers, body) = test_support::get(app, "example.com", &small, &[]).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
}

#[tokio::test]
async fn tenant_max_width_is_enforced() {
    let (addr, _counter) = test_support::spawn_upstream().await;
    let (app, _dir) = test_support::test_app(allow_all(), vec![limited_tenant(1920, Some(2000))]);
    let too_wide = format!(
        "/resize/w/2000/h/10/u/http://127.0.0.1:{}/img.png",
        addr.port()
    );
    let (status, _headers, body) =
        test_support::get(app.clone(), "dim.example.com", &too_wide, &[]).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "{}",
        String::from_utf8_lossy(&body)
    );
    assert!(String::from_utf8_lossy(&body).contains("DIMENSION_TOO_LARGE"));
    assert!(String::from_utf8_lossy(&body).contains("2000 > 1920"));

    let template = format!("/resize/t/wide/u/http://127.0.0.1:{}/img.png", addr.port());
    let (status, _headers, body) =
        test_support::get(app.clone(), "dim.example.com", &template, &[]).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "{}",
        String::from_utf8_lossy(&body)
    );
    assert!(String::from_utf8_lossy(&body).contains("2000 > 1920"));

    let ok = format!(
        "/resize/w/1000/h/10/u/http://127.0.0.1:{}/img.png",
        addr.port()
    );
    let (status, _headers, body) = test_support::get(app, "dim.example.com", &ok, &[]).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
}

#[tokio::test]
async fn global_max_width_wins_over_a_looser_tenant_cap() {
    let (addr, _counter) = test_support::spawn_upstream().await;
    let mut cfg = allow_all();
    cfg.max_width = 4096;
    let (app, _dir) = test_support::test_app(cfg, vec![limited_tenant(5000, None)]);
    let path = format!(
        "/resize/w/4500/h/10/u/http://127.0.0.1:{}/img.png",
        addr.port()
    );
    let (status, _headers, body) = test_support::get(app, "dim.example.com", &path, &[]).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "{}",
        String::from_utf8_lossy(&body)
    );
    assert!(String::from_utf8_lossy(&body).contains("4500 > 4096"));
}

#[tokio::test]
async fn health_ignores_unknown_host() {
    let mut cfg = ServerConfig::default();
    cfg.unknown_host_policy = "reject".to_string();
    let (app, _dir) = test_support::test_app(cfg, vec![]);
    let (status, _headers, body) =
        test_support::get(app, "nobody.example.com", "/health", &[]).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
}
