# Speedwholes

Speedwholes is an on-demand image resizing proxy. A client asks for a size, a fit, and a source; the server fetches that source (only if the tenant's policy allows it), resizes it, and caches the result so the next request is a disk read. One process serves many sites: the HTTP `Host` header picks the tenant, and each tenant has its own source allowlist, size limits, named templates, and optional HMAC signatures. It is the piece you put in front of an object store or origin when you want clients to download the size they will display instead of the original file.

## Features

- Output formats: JPEG, PNG, WebP, and single-frame GIF. Omit the format and the output matches the source.
- Named templates (presets) and a default template, so a short URL can stand in for width, height, fit, quality, and format.
- Fit modes `contain`, `cover`, and `fill`. A width or height of `0` keeps the source aspect ratio. `cover` crops to the target aspect first, then resizes to the exact size.
- A scaled JPEG is resized from the exact picture inside the decoded buffer. Output width and height stay the same. Pixels can differ by less than one decoded pixel from files written by older builds; cached files keep those pixels until they expire.
- Disk cache for the fetched source and for the resized output. Responses send `X-Speedwholes-Cache: hit` or `miss`. Identical requests that overlap while a result is still being produced share that work; the waiting response also sends `X-Speedwholes-Coalesced: 1`. `nocache` and bypass do not join.
- Optional reduced JPEG copies of large JPEG sources (`source_proxy_min_side`, off by default). Later sizes that fit inside the copy are resized from it. Output width, height, URLs, formats, and headers stay the same. Pixels can differ slightly from the direct path while the feature is on.
- Per-tenant HMAC-SHA256 URL signing. A signature is required only when that tenant says so.
- Multi-tenant by `Host`. `images.example.com` loads the tenant named `images`.
- Source allowlists, with private and reserved addresses blocked at connect time and again on every redirect.
- An admin HTTP API and a Vue admin UI.

## Quick start

You need Rust (stable, via rustup), a C compiler, CMake, NASM, pkg-config, and OpenSSL. NASM and CMake are required because the JPEG codec builds libjpeg-turbo from source. There is no system-libjpeg shortcut. Node.js is only needed for the admin UI, not for this resize demo.

Debian or Ubuntu:

```bash
sudo apt-get update
sudo apt-get install -y build-essential cmake nasm pkg-config openssl
```

From a checkout of this repository:

```bash
cd "$(git rev-parse --show-toplevel)"
cd server
cargo build --locked
```

The demo below serves a fixture JPEG on `127.0.0.1` and resizes it through a tenant on `images.example.com`. `allow_private_networks` is `true` in this demo config only, because the origin is loopback. Leave that flag `false` for any config that faces the network. The `admin_token` value is a placeholder, not a production secret.

```bash
cd "$(git rev-parse --show-toplevel)"
ROOT="$(pwd)"
mkdir -p dev-config/certs dev-config/subdomainConfig dev-config/origin dev-config/cache
cp server/tests/fixtures/medium_photo.jpg dev-config/origin/photo.jpg
openssl req -x509 -newkey rsa:2048 \
  -keyout "$ROOT/dev-config/certs/key.pem" \
  -out "$ROOT/dev-config/certs/cert.pem" \
  -days 365 -nodes \
  -subj "/CN=images.example.com" \
  -addext "subjectAltName=DNS:images.example.com,DNS:signed.example.com,DNS:localhost,IP:127.0.0.1"
chmod 600 "$ROOT/dev-config/certs/key.pem"
cat > dev-config/speedwholes.config.json <<EOF
{
  "port": 3000,
  "cache_path": "${ROOT}/dev-config/cache",
  "max_width": 4096,
  "max_height": 4096,
  "allow_all_sources": false,
  "allow_private_networks": true,
  "allowed_sources": [],
  "unknown_host_policy": "reject",
  "trust_forwarded_host": false,
  "max_source_bytes": 52428800,
  "admin_token": "dev-only-change-me",
  "tls_cert": "${ROOT}/dev-config/certs/cert.pem",
  "tls_key": "${ROOT}/dev-config/certs/key.pem"
}
EOF
cat > dev-config/subdomainConfig/images.json <<EOF
{
  "name": "images",
  "source_template": "http://127.0.0.1:18080/{0}",
  "allowed_sources": ["127.0.0.1"],
  "signing_required": false,
  "default_template": "thumb",
  "templates": [
    {
      "name": "thumb",
      "width": 200,
      "height": 200,
      "fit": "cover",
      "quality": 80,
      "format": "jpg"
    }
  ]
}
EOF
cat > dev-config/subdomainConfig/signed.json <<EOF
{
  "name": "signed",
  "allowed_sources": ["127.0.0.1"],
  "signing_required": true,
  "signing_key": "example-signing-key"
}
EOF
```

`dev-config/` is gitignored. Start the origin in one terminal and the server in another:

```bash
cd "$(git rev-parse --show-toplevel)"
python3 -m http.server 18080 --bind 127.0.0.1 --directory dev-config/origin
```

```bash
cd "$(git rev-parse --show-toplevel)/server"
cargo run --locked -- --config-dir ../dev-config --log-format text
```

The server speaks HTTPS only. A self-signed certificate is expected here, so the client uses `-k`. The first resize:

```bash
curl -sk --resolve images.example.com:3000:127.0.0.1 \
  -D - -o /tmp/speedwholes-thumb.jpg \
  "https://images.example.com:3000/resize/w/400/h/0/fit/contain/u/http://127.0.0.1:18080/photo.jpg"
```

That returns `200` and `content-type: image/jpeg`. `X-Speedwholes-Cache` is `miss` the first time and `hit` if you repeat it. Health check, which does not depend on the Host tenant:

```bash
curl -sk https://127.0.0.1:3000/health
```

The body is `OK`.

`server/setup-ssl.sh` is an interactive menu for the same kind of certificate. The `openssl req` command above is the non-interactive version used by this quick start.

## URL format

Resize URLs are `GET /resize/` plus key/value pairs, then a source. Keys:

| Key | Meaning |
| --- | --- |
| `w`, `h` | Width and height in pixels. `0` keeps aspect ratio. Both are required unless a template supplies them. |
| `fit` | `contain`, `cover`, or `fill`. Default `cover` when the key is omitted. |
| `q` | JPEG and WebP quality, 1–100. Default 85. Ignored for PNG and GIF. |
| `f` | `jpg`, `jpeg`, `png`, `webp`, or `gif`. Default is the source format. |
| `t` | Template name. The template replaces `w`, `h`, `fit`, `q`, and `f`. |
| `b` | `1` returns the original bytes after an image sniff, with no resize. |
| `s` | URL signature. See [Signing](#signing). |
| `nocache` | `1` skips the cache. |
| `u` | The rest of the path is the source URL. |
| `p` | The rest of the path is slash-separated parts filled into `source_template`. |

A leading extra slash is accepted (`/resize//...`). `%2F` inside a part stays part of that value; it is not a separator. These examples use the quick-start config. Add `--resolve images.example.com:3000:127.0.0.1` (or `signed.example.com`) to every `curl`.

Contain, width 400, height from the aspect ratio:

```bash
curl -sk --resolve images.example.com:3000:127.0.0.1 \
  -o /tmp/speedwholes-contain.jpg \
  "https://images.example.com:3000/resize/w/400/h/0/fit/contain/u/http://127.0.0.1:18080/photo.jpg"
```

Cover and fill need both dimensions:

```bash
curl -sk --resolve images.example.com:3000:127.0.0.1 \
  -o /tmp/speedwholes-cover.jpg \
  "https://images.example.com:3000/resize/w/320/h/180/fit/cover/u/http://127.0.0.1:18080/photo.jpg"
curl -sk --resolve images.example.com:3000:127.0.0.1 \
  -o /tmp/speedwholes-fill.jpg \
  "https://images.example.com:3000/resize/w/320/h/180/fit/fill/u/http://127.0.0.1:18080/photo.jpg"
```

WebP and PNG:

```bash
curl -sk --resolve images.example.com:3000:127.0.0.1 \
  -D - -o /tmp/speedwholes-out.webp \
  "https://images.example.com:3000/resize/w/400/h/0/fit/contain/f/webp/q/80/u/http://127.0.0.1:18080/photo.jpg"
curl -sk --resolve images.example.com:3000:127.0.0.1 \
  -D - -o /tmp/speedwholes-out.png \
  "https://images.example.com:3000/resize/w/400/h/0/fit/contain/f/png/u/http://127.0.0.1:18080/photo.jpg"
```

The template named `thumb` (200×200, cover, JPEG quality 80). A doubled slash is intentional:

```bash
curl -sk --resolve images.example.com:3000:127.0.0.1 \
  -o /tmp/speedwholes-template.jpg \
  "https://images.example.com:3000/resize//t/thumb/u/http://127.0.0.1:18080/photo.jpg"
```

With `default_template` set, omitting `w` and `h` uses that template:

```bash
curl -sk --resolve images.example.com:3000:127.0.0.1 \
  -o /tmp/speedwholes-default.jpg \
  "https://images.example.com:3000/resize/u/http://127.0.0.1:18080/photo.jpg"
```

Parts mode fills `source_template`. For the `images` tenant that template is `http://127.0.0.1:18080/{0}`, so the part `photo.jpg` becomes the origin path:

```bash
curl -sk --resolve images.example.com:3000:127.0.0.1 \
  -o /tmp/speedwholes-parts.jpg \
  "https://images.example.com:3000/resize/w/320/h/180/fit/contain/p/photo.jpg"
```

Bypass returns the original file. The bytes match `dev-config/origin/photo.jpg`:

```bash
curl -sk --resolve images.example.com:3000:127.0.0.1 \
  -o /tmp/speedwholes-bypass.jpg \
  "https://images.example.com:3000/resize/b/1/u/http://127.0.0.1:18080/photo.jpg"
```

A source that is not on the tenant allowlist is `403`. An unknown Host is `404`:

```bash
curl -sk -o /dev/null -w "%{http_code}\n" --resolve images.example.com:3000:127.0.0.1 \
  "https://images.example.com:3000/resize/w/64/h/64/fit/contain/u/https://example.com/photo.jpg"
curl -sk -o /dev/null -w "%{http_code}\n" --resolve unknown.example.com:3000:127.0.0.1 \
  "https://unknown.example.com:3000/resize/w/64/h/64/fit/contain/u/http://127.0.0.1:18080/photo.jpg"
```

### Signing

When a tenant has a `signing_key`, a provided `s` value is checked. When `signing_required` is true, a missing signature is `403`, and a missing key is `500`. The signed string is the parameters sorted by key, then the source, with `s` itself left out. For the URL below the canonical string is `/fit/contain/h/0/w/200/http://127.0.0.1:18080/photo.jpg`. The signature is the first 16 characters of base64url (no padding) HMAC-SHA256.

```bash
SIG="$(python3 - <<'PY'
import hmac, hashlib, base64
key = b"example-signing-key"
path = "/fit/contain/h/0/w/200/http://127.0.0.1:18080/photo.jpg"
digest = hmac.new(key, path.encode(), hashlib.sha256).digest()
print(base64.urlsafe_b64encode(digest).decode().rstrip("=")[:16])
PY
)"
curl -sk --resolve signed.example.com:3000:127.0.0.1 \
  -o /tmp/speedwholes-signed.jpg \
  "https://signed.example.com:3000/resize/w/200/h/0/fit/contain/s/${SIG}/u/http://127.0.0.1:18080/photo.jpg"
```

The same request without `s` is rejected. The admin API can also build a signature: `POST /config007/sign` with the admin bearer token. See [server/README.md](server/README.md).

## Configuration

`--config-dir` defaults to `/etc/speedwholes`. The global file is `$CONFIG_DIR/speedwholes.config.json`. Tenant files are `$CONFIG_DIR/subdomainConfig/<name>.json`. Unknown JSON keys are ignored. A production-shaped global file, with the secure defaults written out:

```json
{
  "port": 443,
  "cache_path": "/var/cache/speedwholes",
  "max_width": 4096,
  "max_height": 4096,
  "timeout": 30,
  "max_source_pixels": 100000000,
  "max_source_bytes": 52428800,
  "allowed_sources": ["images.example.com"],
  "allow_all_sources": false,
  "allow_private_networks": false,
  "unknown_host_policy": "reject",
  "trust_forwarded_host": false,
  "admin_token": "CHANGE-ME-generate-with-openssl-rand-hex-32",
  "tls_cert": "/etc/letsencrypt/live/images.example.com/fullchain.pem",
  "tls_key": "/etc/letsencrypt/live/images.example.com/privkey.pem"
}
```

Generate the admin token with `openssl rand -hex 32` and keep the file mode `600`. Do not commit it. `**/speedwholes.config.json` is gitignored.

### Global settings

| Key | Default | What it does |
| --- | --- | --- |
| `port` | `3000` | Listen port. The listener is HTTPS. |
| `cache_path` | `./cache` | Directory for the source and resize caches. |
| `max_width` | `4096` | Largest output width. The effective limit is the smaller of this and the tenant value. |
| `max_height` | `4096` | Largest output height. Same rule as `max_width`. |
| `timeout` | `30` | Upstream fetch timeout, in seconds. |
| `max_source_pixels` | `100000000` | Reject a source whose decoded pixel count exceeds this. |
| `max_source_bytes` | `52428800` | Reject an upstream body larger than 50 MiB. |
| `max_concurrent_resizes` | `min(cpus, 8)` | How many decodes, resizes, and encodes may run at once. |
| `source_proxy_min_side` | `0` | Off at `0`. When set, store a reduced JPEG of a large JPEG source whose long side is at least this many pixels, and resize later outputs from that copy when they fit. Non-JPEG sources are unchanged. |
| `source_proxy_quality` | `90` | JPEG quality of that reduced copy. Clamped to 1–100. |
| `source_proxy_max_fill_percent` | `85` | Largest output, as a percentage of the reduced copy, that is made from it. A larger output uses the original. Clamped to 0–100. |
| `allowed_sources` | `[]` | Global source patterns. Used when the tenant list is empty or absent. An empty list allows nothing unless `allow_all_sources` is true. |
| `allow_all_sources` | `false` | Allow any public host for tenants that do not set their own list. A non-empty tenant list still wins. Private addresses stay blocked. |
| `allow_private_networks` | `false` | Allow loopback, private, link-local, and other reserved addresses. For a local origin, tests, or benchmarks. |
| `unknown_host_policy` | `reject` | `reject` returns 404 for a Host with no tenant file. `global` applies the global limits and allowlist and does not require a signature. |
| `trust_forwarded_host` | `false` | When true, `Forwarded` and then `X-Forwarded-Host` select the tenant. When false, only `Host` (or the request URI authority) is used. |
| `source_compression` | `false` | Ask the origin for compressed response bodies. |
| `pool_max_idle_per_host` | `8` | Idle upstream connections kept per host. |
| `admin_token` | unset | Bearer token for `/config007/*`. Unset means those routes are open. Set it. |
| `tls_cert` | required | PEM certificate chain. The process exits if this cannot be loaded. |
| `tls_key` | required | PEM private key. There is no HTTP listener and no HTTP fallback. |
| `disk_min_free_mb` | `500` | Start cache eviction when free space falls below this. `0` disables the check. |
| `disk_check_interval_secs` | `60` | How often free space is checked. |
| `subdomains` | (directory) | Not a key in the global file. Loaded from `subdomainConfig/*.json`. |

CLI flags `--port` and `--cache-path` override the file. `--log-format` is `json` or `text` (default `json`).

### Per-tenant settings

The file name is the tenant name (`images.json`). The `Host` label before the registrable domain must match: `images.example.com` and `images.example.com:3000` both select `images`. An apex such as `example.com` matches no tenant. An IP address or `localhost` selects a tenant named `DEVLOCAL` only when that file exists.

| Key | Default | What it does |
| --- | --- | --- |
| `name` | (required) | Tenant name. Must match the file and the Host label. |
| `max_width` | unset | Optional cap. Effective width is `min(global, tenant)`. Unset means the global cap. |
| `max_height` | unset | Same for height. |
| `source_template` | unset | Pattern for `/p/` parts. `{0}`, `{1}`, … are the parts in order. |
| `signing_key` | unset | HMAC secret. If it is set, any `s` parameter is verified. |
| `signing_required` | `false` | Reject unsigned resize URLs. `true` with no key returns 500. |
| `allowed_sources` | unset | Source patterns. A non-empty list is the only list that applies, even when `allow_all_sources` is true. Empty or omitted means "not set", and the global list is used. |
| `allow_bypass` | allow | `false` disables `/b/1` for this tenant. Omit the key, or set `true`, to allow bypass. |
| `templates` | unset | Named presets. See below. |
| `default_template` | unset | Template used when the URL omits `w` or `h` and does not name a template. |

Pattern forms for `allowed_sources`: `images.example.com` (exact host), `*.example.com` (that host and its subdomains), `images.example.com/files/*` (host plus a path prefix).

A template object has `name`, optional `width`, `height`, `fit`, `quality`, `format`, and `source_template`. A template named from the URL must include width and height. Its `source_template` overrides the tenant template for that request only.

## Security model

Sources are denied unless a policy allows them. With the defaults (`allow_all_sources` false, empty `allowed_sources`, `allow_private_networks` false), a resize cannot fetch anything.

- A non-empty tenant `allowed_sources` is the policy for that tenant. The global list and `allow_all_sources` do not widen it.
- An empty tenant list is not a deny-all. It falls back to the global list, and to `allow_all_sources` if you turned that on.
- `allow_all_sources` allows public hosts only. It does not allow private networks and it does not skip the tenant list.
- Private and reserved addresses are blocked unless `allow_private_networks` is true. That includes loopback, RFC1918, link-local, CGNAT, and the IPv6 equivalents, plus names `localhost` and `*.localhost`. The check runs on the address the resolver returns at connect time, so a public name that points at a private address is still blocked.
- Redirects are not followed blindly. Each hop is checked again, up to five hops. The scheme must be `http` or `https`.
- Signing is per tenant. Required signatures use the canonical string described above. Only successful verifications are cached.
- `unknown_host_policy: reject` (the default) returns 404 for a Host with no tenant file, so an unknown name cannot use the global allowlist.
- `trust_forwarded_host` defaults to false, so a client cannot pick a tenant by sending `X-Forwarded-Host`.
- Bypass (`/b/1`) sniffs the body and only serves JPEG, PNG, GIF, WebP, or BMP. Anything else is 422. The `Content-Type` is the sniffed type. A tenant can set `allow_bypass` to false.
- Every response sends `X-Content-Type-Options: nosniff`. Resize responses also send `Content-Security-Policy: default-src 'none'; sandbox`.
- Upstream failures return a generic body. The origin's error text stays in the log. A handler panic returns a generic 500.
- Set `admin_token`. The admin UI keeps its session token in `localStorage` and sends it as `Authorization: Bearer`.

## Deployment

The image server terminates TLS itself. It does not listen on HTTP. The admin UI is a static Vue build that you serve separately.

Build a release binary and the UI:

```bash
cd server && cargo build --locked --release
cd ../www && npm ci && npm run build
```

The binary is `server/target/release/speedwholes`. The UI files are `www/dist/`. A typical layout is `/usr/local/bin/speedwholes`, `/etc/speedwholes/speedwholes.config.json`, `/etc/speedwholes/subdomainConfig/`, `/var/cache/speedwholes/`, and `/var/www/speedwholes/dist/`.

### systemd

[`systemd-examples/speedwholes.service`](systemd-examples/speedwholes.service) runs the binary as user `speedwholes`, with `--config-dir /etc/speedwholes`, a restart policy, and `AmbientCapabilities=CAP_NET_BIND_SERVICE` so it can bind port 443 without running as root. Install and start:

```bash
sudo cp systemd-examples/speedwholes.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now speedwholes
```

Logs go to the journal (`journalctl -u speedwholes`). The unit's `ReadWritePaths` must include your `cache_path`. More of the install is in [docs/deployment.md](docs/deployment.md) and [systemd-examples/README.md](systemd-examples/README.md).

### TLS

Point `tls_cert` and `tls_key` at a PEM chain and key. For Let's Encrypt, leave `privkey*.pem` at mode `600` and grant the service user a read ACL instead of making the key world-readable:

```bash
sudo ./scripts/install.sh --upgrade --tls-domain images.example.com
```

That script only changes the certificate lineage you name. If `setfacl` is missing, install the `acl` package. Another option is systemd `LoadCredential=`, documented in [docs/deployment.md](docs/deployment.md): the key stays root-only and the service reads it from `$CREDENTIALS_DIRECTORY`. After a renewal, restart the service so it loads the new files.

### nginx for the admin SPA

nginx serves the built admin UI. It does not terminate TLS for the image server. Copy `www/dist/` to `/var/www/speedwholes/dist/` and write `config.json` there so the UI knows the API origin (`https://images.example.com`, or whichever host the binary uses). A site file:

```nginx
server {
    listen 443 ssl;
    server_name admin.example.com;

    ssl_certificate     /etc/letsencrypt/live/admin.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/admin.example.com/privkey.pem;

    root /var/www/speedwholes/dist;
    index index.html;

    location / {
        try_files $uri $uri/ /index.html;
    }
}
```

```bash
sudo nginx -t && sudo systemctl reload nginx
```

Leave `trust_forwarded_host` false. The image server sees the `Host` the client used. Turn the flag on only when a proxy in front rewrites `Host` and you have decided to trust `X-Forwarded-Host`.

## Development and tests

[`startLocalDev.sh`](startLocalDev.sh) starts the API and the Vite dev server in tmux. It writes `dev-config/` on first run (certificate, config from `server/speedwholes.config.example.json`, local paths) and passes `--config-dir`. The UI is at `http://127.0.0.1:8081`. The API is `https://localhost:3000`.

Run the server tests, the linters, and the UI build from a checkout:

```bash
cd server
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --check
cd ../www
npm ci
npm run build
```

Shell scripts:

```bash
shellcheck -S error deploy.sh startLocalDev.sh scripts/*.sh server/*.sh benchmarks/*.sh
```

A one-shot local benchmark (the harness serves its own fixtures; it is not a published result):

```bash
bash benchmarks/run.sh --serve-fixtures --iterations 1 --concurrency 1 --sizes 100
```

Add a test for every behaviour change. Commit messages look like `fix(security): …`, `docs: …`, or `chore: …`. See [CONTRIBUTING.md](CONTRIBUTING.md).

## Benchmarks

No published results yet.

## Limitations

- No AVIF encode or decode path. Output formats are JPEG, PNG, WebP, and GIF.
- EXIF orientation is not applied. A JPEG tagged as rotated is resized in stored pixel order.
- GIF output is a single frame. Animation is not preserved.
- Quality applies to JPEG and WebP. PNG and GIF are encoded losslessly and ignore `q`.
- The process requires a TLS certificate and key. It will not start on plain HTTP.
- Bypass will not pass through HTML, SVG, or other non-image bodies.

## License

Speedwholes is released under the [MIT License](LICENSE). Copyright (c) 2026 clarkbox (https://github.com/clarkbox).

C libraries linked into the server binary are listed in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

Contributions are under the same license. Read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request, and [SECURITY.md](SECURITY.md) before reporting a vulnerability.

## Further reading

- [Backend API and admin routes](server/README.md)
- [Admin UI](www/README.md)
- [Deployment](docs/deployment.md)
- [Development](docs/development.md)
- [Changelog](CHANGELOG.md)
