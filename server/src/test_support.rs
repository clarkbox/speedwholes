//! In-process fixtures for integration tests.
//!
//! Images are generated with the `image` crate. No binary fixtures are required.

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::{OriginalUri, Query, State};
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get as route_get;
use axum::{body::Body, Router};
use futures::stream;
use http_body_util::BodyExt;
use image::{DynamicImage, ImageBuffer, ImageFormat, Rgb, Rgba};
use tempfile::TempDir;
use tower::ServiceExt;

use crate::app::build_router;
use crate::config::{AppState, ServerConfig, SubdomainConfig, TlsConfig};

/// Counts upstream requests and bytes actually written to the response body.
#[derive(Debug, Default)]
pub struct HitCounter {
    hits: Mutex<HashMap<String, u64>>,
    bytes_written: Mutex<HashMap<String, u64>>,
    seen_paths: Mutex<Vec<String>>,
}

impl HitCounter {
    /// Request count for `path`. Used by later security tests.
    #[allow(dead_code)]
    pub fn hits(&self, path: &str) -> u64 {
        self.hits
            .lock()
            .expect("hit counter")
            .get(path)
            .copied()
            .unwrap_or(0)
    }

    /// Bytes the upstream actually wrote for `path`.
    pub fn bytes_written(&self, path: &str) -> u64 {
        self.bytes_written
            .lock()
            .expect("byte counter")
            .get(path)
            .copied()
            .unwrap_or(0)
    }

    pub fn seen_paths(&self) -> Vec<String> {
        self.seen_paths.lock().expect("seen paths").clone()
    }

    fn record_hit(&self, path: &str) {
        *self
            .hits
            .lock()
            .expect("hit counter")
            .entry(path.to_string())
            .or_insert(0) += 1;
    }

    fn add_bytes(&self, path: &str, n: u64) {
        *self
            .bytes_written
            .lock()
            .expect("byte counter")
            .entry(path.to_string())
            .or_insert(0) += n;
    }

    fn note_path(&self, path: String) {
        self.seen_paths.lock().expect("seen paths").push(path);
    }
}

/// Encode a solid-ish PNG of the requested size.
pub fn png_bytes(w: u32, h: u32) -> Vec<u8> {
    let img: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::from_fn(w, h, |x, y| {
        Rgba([(x % 256) as u8, (y % 256) as u8, 40, 255])
    });
    encode(DynamicImage::ImageRgba8(img), ImageFormat::Png)
}

/// Encode a solid-ish JPEG of the requested size.
pub fn jpeg_bytes(w: u32, h: u32) -> Vec<u8> {
    let img: ImageBuffer<Rgb<u8>, Vec<u8>> =
        ImageBuffer::from_fn(w, h, |x, y| Rgb([(x % 256) as u8, (y % 256) as u8, 80]));
    encode(DynamicImage::ImageRgb8(img), ImageFormat::Jpeg)
}

fn encode(img: DynamicImage, format: ImageFormat) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, format)
        .expect("encode fixture image");
    buf.into_inner()
}

#[derive(Clone)]
struct UpstreamState {
    counter: Arc<HitCounter>,
    png: Bytes,
    jpeg: Bytes,
    tall: Bytes,
    wide: Bytes,
    zero: Bytes,
}

/// Start an axum upstream on `127.0.0.1:0`. The task lives until the test process ends.
pub async fn spawn_upstream() -> (SocketAddr, Arc<HitCounter>) {
    let counter = Arc::new(HitCounter::default());
    let state = UpstreamState {
        counter: Arc::clone(&counter),
        png: Bytes::from(png_bytes(64, 48)),
        jpeg: Bytes::from(jpeg_bytes(32, 24)),
        tall: Bytes::from(png_bytes(1, 10_000)),
        wide: Bytes::from(png_bytes(10_000, 1)),
        zero: Bytes::from(zero_width_png()),
    };

    let app = Router::new()
        .route("/img.png", route_get(serve_png))
        .route("/img.jpg", route_get(serve_jpg))
        .route("/png.jpg", route_get(serve_png_as_jpg))
        .route("/o/*rest", route_get(serve_o))
        .route("/html", route_get(serve_html))
        .route("/redirect", route_get(serve_redirect))
        .route("/loop", route_get(serve_loop))
        .route("/big", route_get(serve_big))
        .route("/big-cl", route_get(serve_big_cl))
        .route("/tall.png", route_get(serve_tall))
        .route("/wide.png", route_get(serve_wide))
        .route("/zero.png", route_get(serve_zero))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            count_hits,
        ))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind upstream");
    let addr = listener.local_addr().expect("upstream addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("upstream serve");
    });
    (addr, counter)
}

async fn count_hits(
    State(state): State<UpstreamState>,
    req: Request<Body>,
    next: axum::middleware::Next,
) -> Response {
    let path = req.uri().path().to_string();
    state.counter.record_hit(&path);
    next.run(req).await
}

async fn serve_png(State(state): State<UpstreamState>) -> Response {
    image_response(state.png, "image/png")
}

async fn serve_jpg(State(state): State<UpstreamState>) -> Response {
    image_response(state.jpeg, "image/jpeg")
}

async fn serve_png_as_jpg(State(state): State<UpstreamState>) -> Response {
    image_response(state.png, "image/jpeg")
}

async fn serve_tall(State(state): State<UpstreamState>) -> Response {
    image_response(state.tall, "image/png")
}

async fn serve_wide(State(state): State<UpstreamState>) -> Response {
    image_response(state.wide, "image/png")
}

async fn serve_zero(State(state): State<UpstreamState>) -> Response {
    image_response(state.zero, "image/png")
}

/// A valid 8×8 PNG whose IHDR width is patched to 0 and whose chunk CRC is recomputed.
fn zero_width_png() -> Vec<u8> {
    let mut png = png_bytes(8, 8);
    assert_eq!(
        &png[12..16],
        b"IHDR",
        "fixture PNG has no IHDR at the expected offset"
    );
    png[16..20].copy_from_slice(&0u32.to_be_bytes());
    let crc = crc32fast::hash(&png[12..29]);
    png[29..33].copy_from_slice(&crc.to_be_bytes());
    png
}

async fn serve_o(State(state): State<UpstreamState>, OriginalUri(uri): OriginalUri) -> Response {
    let path = uri.path().to_string();
    state.counter.note_path(path.clone());
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "image/jpeg")
        .header("x-seen-path", path)
        .body(Body::from(state.jpeg.clone()))
        .expect("o response")
}

async fn serve_html() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        "<html><body>not an image</body></html>",
    )
}

async fn serve_redirect(Query(q): Query<HashMap<String, String>>) -> Response {
    let to = q
        .get("to")
        .cloned()
        .unwrap_or_else(|| "/img.png".to_string());
    Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, to)
        .body(Body::empty())
        .expect("redirect")
}

async fn serve_loop() -> Response {
    Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, "/loop")
        .body(Body::empty())
        .expect("loop")
}

async fn serve_big(State(state): State<UpstreamState>) -> Response {
    streaming_body(&state, "/big", 5 * 1024 * 1024, false)
}

async fn serve_big_cl(State(state): State<UpstreamState>) -> Response {
    streaming_body(&state, "/big-cl", 5 * 1024 * 1024, true)
}

fn streaming_body(
    state: &UpstreamState,
    path: &'static str,
    total: usize,
    lie_about_length: bool,
) -> Response {
    let counter = Arc::clone(&state.counter);
    let stream = stream::unfold(0usize, move |sent| {
        let counter = Arc::clone(&counter);
        async move {
            if sent >= total {
                return None;
            }
            let n = (64 * 1024).min(total - sent);
            counter.add_bytes(path, n as u64);
            let chunk: Result<Bytes, Infallible> = Ok(Bytes::from(vec![0xAB; n]));
            Some((chunk, sent + n))
        }
    });
    let mut builder = Response::builder().status(StatusCode::OK);
    if lie_about_length {
        builder = builder.header(header::CONTENT_LENGTH, "1000000000");
    }
    builder
        .body(Body::from_stream(stream))
        .expect("stream body")
}

fn image_response(bytes: Bytes, content_type: &'static str) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(bytes))
        .expect("image response")
}

/// App wired to a temp cache directory and dummy TLS paths (never opened).
pub fn test_app(mut cfg: ServerConfig, tenants: Vec<SubdomainConfig>) -> (Router, TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    cfg.cache_path = dir.path().join("cache").to_string_lossy().to_string();
    cfg.subdomains = tenants;
    let tls = TlsConfig {
        cert_path: dir.path().join("cert.pem"),
        key_path: dir.path().join("key.pem"),
    };
    let state = AppState::from_config(cfg, dir.path().to_path_buf(), tls);
    let app = build_router(state);
    (app, dir)
}

/// Issue one GET through the router. `app` is consumed; clone it to reuse.
pub async fn get(
    app: Router,
    host: &str,
    path: &str,
    extra_headers: &[(&str, &str)],
) -> (StatusCode, HeaderMap, Bytes) {
    let mut builder = Request::builder()
        .method("GET")
        .uri(path)
        .header(header::HOST, host);
    for (name, value) in extra_headers {
        builder = builder.header(*name, *value);
    }
    let req = builder.body(Body::empty()).expect("request");
    let res = app.oneshot(req).await.expect("oneshot");
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = res.into_body().collect().await.expect("body").to_bytes();
    (status, headers, bytes)
}
