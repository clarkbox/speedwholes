# Third-party notices

Speedwholes statically links the following C libraries into the server binary.
The Rust crates and npm packages this repository depends on are under MIT,
Apache-2.0, BSD, or ISC licenses. A complete machine-readable inventory can be
produced later with `cargo about generate` (follow-up; not generated here).

## libjpeg-turbo

Linked via the `turbojpeg` / `turbojpeg-sys` crates, which build libjpeg-turbo
from source (`cmake` + `nasm`).

- Upstream: https://github.com/libjpeg-turbo/libjpeg-turbo
- License: IJG License, BSD-3-Clause, and zlib License
  (https://github.com/libjpeg-turbo/libjpeg-turbo/blob/main/LICENSE.md)

## libwebp

Linked via `libwebp-sys` 0.14 (the `webp` crate is no longer a dependency).

- Upstream: https://chromium.googlesource.com/webm/libwebp
- License: BSD-3-Clause
  (https://chromium.googlesource.com/webm/libwebp/+/refs/heads/main/COPYING)

## ring

Used by `rustls` for TLS (`rustls` is built with the `ring` crypto provider).

- Upstream: https://github.com/briansmith/ring
- License: ISC-style and OpenSSL-derived portions
  (https://github.com/briansmith/ring/blob/main/LICENSE)
