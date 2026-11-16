# Changelog

## Unreleased

### Breaking / security

- `allow_all_sources` defaults to `false`. A config that omitted the key used to allow any source host. Setting it to `true` is an interim switch only: private and reserved addresses are blocked either way unless `allow_private_networks` is `true`.
- A non-empty tenant `allowed_sources` list is always enforced, including when `allow_all_sources` is `true`. Review every tenant file that has a list before deploying. A stale list starts returning 403. An empty tenant list still means "not set" and falls back to the global list.
- `allow_private_networks` defaults to `false`. Loopback, private, link-local, and other reserved addresses are rejected when the connection is made, and again on every redirect. A local origin, the test suite, or the benchmark harness needs this flag set to `true`. `allow_all_sources: true` alone no longer reaches `127.0.0.1`.
- `unknown_host_policy` defaults to `reject`. A Host with no tenant file, including an apex name and an IP address or `localhost` when no `DEVLOCAL` tenant exists, returns 404 on `/resize`. `/health` is unchanged. Add a tenant file for each host that should keep working, or set the policy to `global`.
- `trust_forwarded_host` defaults to `false`. `Forwarded` and `X-Forwarded-Host` are ignored unless you set it. Leave it false when the image server terminates TLS itself and sees the original `Host`.
- `max_source_bytes` defaults to `52428800` (50 MiB). A larger upstream body returns 422. Raise the cap if an original exceeds 50 MiB.
- `max_concurrent_resizes` defaults to `min(available CPUs, 8)`.
- Tenant `allow_bypass` defaults to allowing `/b/1`. Set it to `false` to disable bypass for that tenant.
- Effective `max_width` and `max_height` are `min(global, tenant)`. A tenant cap below a size you already serve will start failing those requests.
- `signing_required: true` with no `signing_key` returns 500. Add a key or set the flag to `false`.
- Bypass sniffs the body and returns 422 for anything that is not JPEG, PNG, GIF, WebP, or BMP. `Content-Type` is the sniffed type. Older non-image objects in the source cache now return 422 instead of being served.
- Upstream failures and panics return a generic body. The detail is logged, not returned.
- The server listens only with TLS. It does not fall back to HTTP.
- Let's Encrypt private keys must stay mode `600`. Grant the service user access with `scripts/install.sh --upgrade --tls-domain <domain>`, or pass the key with systemd `LoadCredential=`. Do not `chmod` the key world-readable.
- If a `DEVLOCAL.json` tenant is present on a production config directory, delete it. It is a development tenant.
- Confirm `admin_token` is a secret you generated for that deployment, not a placeholder from an example file.
- Rollback of the binary is safe with these keys present: the previous binary ignores unknown JSON fields. Keep a copy of the previous binary before you replace it.

### Fixed

- Redirect hops are checked one by one (scheme, allowlist, private addresses), up to five, instead of being followed automatically.
- `cover` crops the source to the target aspect, then resizes to the requested size.
- Output dimensions are bounded before the image is decoded.
- Decode, resize, and encode run on the blocking pool, limited by `max_concurrent_resizes`.

### Dependencies

- axum 0.8, axum-server 0.9, tower-http 0.6. Route patterns use `{name}` and `{*rest}`. HTTP paths are unchanged.
- sha2 0.11 and hmac 0.13 together. base64 0.23. imagesize 0.15. nix 0.32.
- `rustls-pemfile` left the graph. AVIF encoding stays disabled so the server does not build the unmaintained `paste` crate; Cargo.lock still records it as an optional image dependency.
- Admin UI: vue-router 5.
- `fast_image_resize` 6.1.0.
- `libwebp-sys` 0.14. The `webp` crate was removed.
- `lru` 0.18.
- `aws-lc` left the graph. `axum-server` is built with `tls-rustls-no-provider`, and rustls uses the ring provider.
- Admin UI: Vite 8 and Vue 3.5.
