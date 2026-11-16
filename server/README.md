# Speedwholes Server - Backend API

The backend API server for Speedwholes image processing and configuration management.

## Overview

This is the core image processing engine and **API-only server** that:
- Fetches remote images via HTTP
- Resizes and converts images on-demand
- Caches processed images to disk
- Provides REST API for configuration management
- Supports per-subdomain configuration with URL signing

**Note:** This server provides API endpoints only. For the admin web UI, see the `www` folder which contains a separate web server that serves the Vue.js frontend and proxies API calls to this server.

## Installation

### Build Dependencies

The server requires a C toolchain to compile native image-processing libraries:

| Dependency | Required By | Purpose |
|------------|-------------|---------|
| C compiler (gcc/clang) | `libwebp-sys`, `turbojpeg` | Compile native C libraries |
| CMake | `turbojpeg` | Build libjpeg-turbo from source |
| NASM | `turbojpeg` | SIMD (SSE2/AVX2/NEON) assembly for JPEG |

```bash
# RHEL / CentOS / Fedora
sudo dnf install cmake nasm gcc

# Ubuntu / Debian
sudo apt install cmake nasm build-essential
```

CMake and NASM are required. A system libjpeg package is not used.

### Build

```bash
cargo build --release
```

The binary will be at `target/release/speedwholes`.

## Usage

```bash
# Config directory defaults to /etc/speedwholes. TLS is required.
speedwholes --config-dir /etc/speedwholes

# Custom port and cache path
speedwholes --port 8080 --cache-path /var/cache/speedwholes

# Local config directory
speedwholes --config-dir ../dev-config --log-format text
```

### CLI Options

| Option | Default | Description |
|--------|---------|-------------|
| `--port`, `-p` | config file (`3000`) | Listen port (overrides config file). The listener is HTTPS. |
| `--config-dir` | `/etc/speedwholes` | Directory for config files and the PID lock |
| `--cache-path` | config file (`./cache`) | Disk cache directory (overrides config file) |
| `--log-format` | json | Log output format: `json` (structured) or `text` (human-readable) |

All other settings are configured via the configuration file (`speedwholes.config.json`).

#### Log Formats

- **json** (default): Structured JSON logs, one object per line. Best for production, log aggregation, and automated parsing.
- **text**: Human-readable colored output with timestamps. Best for development and debugging.

```bash
# Development with readable logs
speedwholes --log-format text

# Production with JSON logs (default)
speedwholes --log-format json
```

## API Endpoints

### Health Check

```
GET /health
```

Returns `200 OK` if the service is running.

### Resize Image

```
GET /resize/w/{width}/h/{height}/fit/{mode}/{url}
```

#### Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `width` | u32 | Target width in pixels (0 = auto-calculate from height) |
| `height` | u32 | Target height in pixels (0 = auto-calculate from width) |
| `mode` | string | Fit mode: `contain`, `cover`, or `fill` |
| `url` | string | Full URL of the source image |

#### Optional Parameters

Add these as key/value pairs before the URL:

- `q/{quality}` - JPEG/WebP quality (1-100, default: 85)
- `f/{format}` - Output format: jpg, png, webp, gif
- `nocache/1` - Bypass cache and re-fetch/re-process

#### Fit Modes

- **contain** - Scale to fit within bounds, preserve aspect ratio
- **cover** - Scale to cover bounds, crop excess (center crop)
- **fill** - Stretch to exact dimensions (may distort)

#### Examples

The server is HTTPS-only, and the Host must match a tenant whose allowlist includes the source. Commands that return an image against a local origin are in the [root README](../README.md#url-format). The shapes are:

```text
https://images.example.com/resize/w/200/h/150/fit/cover/u/https://images.example.com/image.jpg
https://images.example.com/resize/w/400/h/0/fit/contain/u/https://cdn.example.com/photo.png
https://images.example.com/resize/w/0/h/300/fit/contain/u/https://images.example.com/img.webp
https://images.example.com/resize/w/400/h/300/f/webp/q/90/u/https://images.example.com/image.jpg
```

#### Response

- **200 OK** - Returns the resized image with appropriate Content-Type
- **400 Bad Request** - Invalid parameters or dimensions exceed max
- **502 Bad Gateway** - Failed to fetch source image
- **422 Unprocessable Entity** - Failed to decode image

Response headers include:
- `Content-Type`: Matches source image format (image/jpeg, image/png, etc.)
- `Cache-Control: public, max-age=31536000` (1 year)

### Admin API

All admin endpoints require authentication via Bearer token in `Authorization` header.

**For web UI access**, use the `www` admin UI. It stores the session token in `localStorage` and sends `Authorization: Bearer`.

#### Get Configuration

```
GET /config007/json
```

Returns the current runtime configuration. This endpoint is read-only.

#### Get Statistics

```
GET /config007/stats
```

Returns a `StatsSnapshot`:
```json
{
  "image_requests": 1234,
  "cache_hits": 890,
  "cache_misses": 344,
  "source_cache_hits": 100,
  "source_cache_misses": 20,
  "signature_cache_hits": 10,
  "signature_cache_misses": 4,
  "last_subdomain": "images"
}
```

#### List Subdomains

```
GET /config007/subdomains
```

Returns array of subdomain configurations.

#### Save Subdomain

```
POST /config007/subdomain/{name}
Content-Type: application/json

{
  "name": "cdn",
  "max_width": 2048,
  "max_height": 2048,
  "signing_key": "secret",
  "signing_required": true,
  "allowed_sources": ["*.example.com"]
}
```

#### Delete Subdomain

```
DELETE /config007/subdomain/{name}
```

#### Sign URL

```
POST /config007/sign
Content-Type: application/json

{
  "subdomain": "cdn",
  "path": "/resize/w/300/h/200/https://example.com/image.jpg"
}
```

Returns signed URL with HMAC signature.

## Configuration

### Configuration Precedence

Configuration values are determined in the following order (highest to lowest priority):

1. **CLI Arguments** - Only `--port` and `--cache-path` can be overridden via CLI
2. **Configuration File** (`speedwholes.config.json` in config directory)
3. **Built-in Defaults** (lowest priority)

### Configuration File

The configuration file is located at `<config-dir>/speedwholes.config.json`:

```json
{
  "port": 3000,
  "cache_path": "./cache",
  "max_width": 4096,
  "max_height": 4096,
  "timeout": 30,
  "max_source_pixels": 100000000,
  "allowed_sources": ["images.example.com"],
  "allow_all_sources": false,
  "admin_token": "your-secret-token",
  "tls_cert": "/path/to/certificate.pem",
  "tls_key": "/path/to/private-key.pem"
}
```

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `port` | u16 | 3000 | HTTPS listen port |
| `cache_path` | string | "./cache" | Disk cache directory |
| `max_width` | u32 | 4096 | Maximum resize width |
| `max_height` | u32 | 4096 | Maximum resize height |
| `timeout` | u64 | 30 | Upstream fetch timeout (seconds) |
| `max_source_pixels` | u64 | 100000000 | Max source image size (100MP) |
| `max_source_bytes` | u64 | 52428800 | Max upstream body (50 MiB) |
| `source_proxy_min_side` | u32 | 0 | Off at 0. Minimum long side of a reduced JPEG kept for large JPEG sources. |
| `source_proxy_quality` | u8 | 90 | JPEG quality of that copy. Clamped to 1–100. |
| `source_proxy_max_fill_percent` | u8 | 85 | Largest output, as a percent of the copy, made from it. Clamped to 0–100. |
| `allowed_sources` | array | [] | Allowed source URL patterns |
| `allow_all_sources` | bool | false | Allow any public host when a tenant has no allowlist. A non-empty tenant `allowed_sources` is always enforced. |
| `allow_private_networks` | bool | false | Allow loopback and other reserved addresses |
| `unknown_host_policy` | string | `reject` | `reject` or `global` |
| `trust_forwarded_host` | bool | false | Honor `Forwarded` / `X-Forwarded-Host` |
| `admin_token` | string | null | Bearer token for admin API. Set it. |
| `tls_cert` | string | required | PEM certificate chain. The process exits without it. |
| `tls_key` | string | required | PEM private key. There is no HTTP fallback. |

The full key list, including `max_concurrent_resizes`, `source_compression`, `pool_max_idle_per_host`, `disk_min_free_mb`, and `disk_check_interval_secs`, is in the [root README](../README.md#global-settings).

**Note**: The `admin_token` protects `/config007/*`. Set it to a secure random string.

**TLS**: Both `tls_cert` and `tls_key` are required. See [TLS/HTTPS Configuration](#tlshttps-configuration).

### Subdomain Configuration

Per-subdomain settings are stored in separate JSON files in `<config-dir>/subdomainConfig/`:

```json
{
  "name": "cdn",
  "max_width": 2048,
  "max_height": 2048,
  "source_template": "https://cdn.example.com/{1}/{2}.{0}?token={3}",
  "signing_key": "your-hmac-secret",
  "signing_required": true,
  "allowed_sources": ["*.example.com", "cdn.mysite.com/*"]
}
```

#### Source URL Modes

There are two ways to specify the source image in resize URLs:

**URL mode (`/u/`)** - Direct URL:
```
/resize/w/300/h/200/u/https://example.com/images/photo.jpg
```
Everything after `/u/` is used as the literal source URL.

**Parts mode (`/p/`)** - Template interpolation:
```
/resize/w/300/h/200/p/jpg/photos/image123/mytoken
```
The parts after `/p/` are split by `/` and interpolated into the subdomain's `source_template`:
- `{0}` → `jpg`
- `{1}` → `photos`
- `{2}` → `image123`
- `{3}` → `mytoken`

With template `https://cdn.example.com/{1}/{2}.{0}?token={3}`, this resolves to:
```
https://cdn.example.com/photos/image123.jpg?token=mytoken
```

**Legacy mode** - Direct `https://` still works for backwards compatibility:
```
/resize/w/300/h/200/https://example.com/images/photo.jpg
```

## Multiple Instances

You can run multiple isolated instances on different ports, each with their own configuration:

```bash
# Instance 1 - production on port 3000
speedwholes \
  --port 3000 \
  --config-dir /var/lib/speedwholes/prod \
  --cache-path /var/cache/speedwholes

# Instance 2 - staging on port 3001 (sharing cache is safe and efficient)
speedwholes \
  --port 3001 \
  --config-dir /var/lib/speedwholes/staging \
  --cache-path /var/cache/speedwholes
```

**Instance Locking**: Each `config-dir` can only have one running instance. If you try to start a second instance with the same config directory, it will exit with an error.

**Shared Cache**: Multiple instances can safely share the same `cache-path` directory. The cache uses atomic write operations.

## Architecture

```
┌─────────┐     ┌──────────────┐     ┌─────────────┐
│ Client  │────▶│  Speedwholes  │────▶│ Disk Cache  │
└─────────┘     └──────────────┘     └─────────────┘
                       │
                       ▼
                ┌─────────────┐
                │ Remote URL  │
                └─────────────┘
```

1. Request comes in with resize parameters and source URL
2. Check disk cache for pre-resized image
3. If cache miss: fetch source image, resize, cache result
4. Return resized image

### Image Processing Pipeline

JPEG images are decoded and encoded using **turbojpeg** (libjpeg-turbo), which provides SIMD-accelerated (SSE2/AVX2/NEON) JPEG operations -- typically 2-5x faster than pure-Rust codecs. Non-JPEG formats (PNG, GIF, WebP, BMP) use the `image` crate. WebP encoding uses `libwebp` for lossy output. Resizing is handled by `fast_image_resize` with SIMD-optimized convolution filters.

### Cache Key Generation

Cache keys are SHA256 hashes of: `url + width + height + fit_mode + format + quality`

Cache files are organized in subdirectories by the first 2 characters of the hash for filesystem performance.

## CORS Configuration

The server allows all origins by default (`Access-Control-Allow-Origin: *`). This enables:
- Cross-origin requests from any admin UI or client application
- Canvas-based image operations that require `crossOrigin = 'anonymous'`

Security is enforced via:
- **Signing keys** for URL tampering prevention
- **Admin authentication** for protected endpoints
- **Source filters** for controlling which origins can be proxied

## TLS/HTTPS Configuration

Speedwholes supports native HTTPS via rustls. When TLS is configured, the server automatically starts in HTTPS mode.

### Quick Setup with Script

The easiest way to configure TLS is using the included setup script:

```bash
./setup-ssl.sh
```

This interactive script will:
- Guide you through choosing self-signed or Let's Encrypt certificates
- Generate or obtain certificates automatically
- Update your `speedwholes.config.json` with the correct paths
- Set up certificate renewal hooks (for Let's Encrypt)

**Requirements:** `jq` and `openssl` (for self-signed) or `certbot` (for Let's Encrypt)

### Manual Configuration

If you prefer manual setup, follow the sections below.

### Enabling TLS

Add `tls_cert` and `tls_key` to your `speedwholes.config.json`:

```json
{
  "port": 443,
  "tls_cert": "/etc/speedwholes/certs/certificate.pem",
  "tls_key": "/etc/speedwholes/certs/private-key.pem",
  ...
}
```

Both paths must be set and point to valid PEM files. The server listens with TLS only. If either file is missing or invalid, the process exits. There is no HTTP fallback.

### Self-Signed Certificates (Development)

For local development, create self-signed certificates using OpenSSL:

```bash
# Create a directory for certificates
mkdir -p certs

# Generate a self-signed certificate (valid for 365 days)
openssl req -x509 -newkey rsa:4096 -keyout certs/key.pem -out certs/cert.pem -days 365 -nodes \
  -subj "/CN=localhost" \
  -addext "subjectAltName=DNS:localhost,IP:127.0.0.1"

# Set permissions
chmod 600 certs/key.pem
```

Then configure:

```json
{
  "port": 3000,
  "tls_cert": "./certs/cert.pem",
  "tls_key": "./certs/key.pem"
}
```

**Note:** Browsers will show a security warning for self-signed certificates. You can bypass this in development or use `mkcert` for locally-trusted certificates:

```bash
# Install mkcert (https://github.com/FiloSottile/mkcert)
# macOS: brew install mkcert
# Linux: See mkcert installation docs

# Create local CA (one-time setup)
mkcert -install

# Generate trusted local certificates
mkcert -key-file certs/key.pem -cert-file certs/cert.pem localhost 127.0.0.1
```

### Let's Encrypt Certificates (Production)

For production, use [Let's Encrypt](https://letsencrypt.org/) with [certbot](https://certbot.eff.org/) to obtain free, trusted certificates.

#### Option 1: Standalone Mode (server must be stopped)

```bash
# Install certbot
sudo apt install certbot

# Obtain certificate (server must not be running on port 80)
sudo certbot certonly --standalone -d api.yourdomain.com

# Certificates are saved to:
# /etc/letsencrypt/live/api.yourdomain.com/fullchain.pem
# /etc/letsencrypt/live/api.yourdomain.com/privkey.pem
```

Configure speedwholes:

```json
{
  "port": 443,
  "tls_cert": "/etc/letsencrypt/live/api.yourdomain.com/fullchain.pem",
  "tls_key": "/etc/letsencrypt/live/api.yourdomain.com/privkey.pem"
}
```

**Important:** The speedwholes user needs read access to the certificate files:

```bash
# Add speedwholes user to the ssl-cert group (Debian/Ubuntu)
sudo usermod -aG ssl-cert speedwholes

# Or adjust permissions on the live directory
sudo chmod 750 /etc/letsencrypt/live/api.yourdomain.com
sudo chgrp speedwholes /etc/letsencrypt/live/api.yourdomain.com
```

#### Option 2: Webroot Mode (server keeps running)

If you have an existing HTTP server or want to use port 80 for verification:

```bash
# Create webroot directory
sudo mkdir -p /var/www/acme-challenge

# Run certbot with webroot
sudo certbot certonly --webroot -w /var/www/acme-challenge -d api.yourdomain.com
```

#### Certificate Renewal

Let's Encrypt certificates expire after 90 days. Set up auto-renewal:

```bash
# Test renewal
sudo certbot renew --dry-run

# Enable auto-renewal timer (usually enabled by default)
sudo systemctl enable certbot.timer
sudo systemctl start certbot.timer
```

**Certificate Reload:** When certificates are renewed, you need to restart speedwholes to pick up the new certificates:

```bash
# Add to /etc/letsencrypt/renewal-hooks/post/speedwholes.sh
#!/bin/bash
systemctl restart speedwholes
```

```bash
sudo chmod +x /etc/letsencrypt/renewal-hooks/post/speedwholes.sh
```

### Running on Port 443

To bind to port 443 (standard HTTPS port), you either need:

1. **Run as root** (not recommended)
2. **Use setcap** to grant the binary capability:
   ```bash
   sudo setcap 'cap_net_bind_service=+ep' /usr/local/bin/speedwholes
   ```
3. **Serve the admin UI** with nginx. The image server always terminates its own TLS; do not proxy `/resize` to a plain HTTP port. See the [root README](../README.md#nginx-for-the-admin-spa).

### TLS Configuration in SystemD

Update your systemd service file for HTTPS:

```ini
[Service]
# ... existing config ...

# Grant access to certificate files
ReadOnlyPaths=/etc/letsencrypt

# If using port 443 with non-root user
AmbientCapabilities=CAP_NET_BIND_SERVICE
```

## Development

```bash
# Watch and auto-reload on changes
cargo watch -x run

# With custom args
cargo watch -x 'run -- --port 3000'

# Clear screen between runs
cargo watch -c -x run

# Run tests
cargo test

# Run benchmarks
cargo bench
```

## Logging

Set the `RUST_LOG` environment variable to control log verbosity:

```bash
RUST_LOG=debug speedwholes      # Verbose logging
RUST_LOG=speedwholes=info speedwholes  # Default
RUST_LOG=error speedwholes      # Errors only
```

## License

MIT
