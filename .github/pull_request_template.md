## What changed

## How you tested

- [ ] `cargo test --locked` in `server/`
- [ ] `cargo clippy --locked --all-targets -- -D warnings`
- [ ] `cargo fmt --check`
- [ ] `npm ci && npm run build` in `www/`, if the UI changed

## Notes

Add a test for every behaviour change. Do not include signing keys, admin tokens, or generated certificates.
