# Contributing

Thanks for looking at Speedwholes. The project is MIT licensed. Contributions are accepted under the same license (inbound = outbound). The copyright holder is clarkbox (https://github.com/clarkbox).

## Prerequisites

- Rust stable, installed with rustup, including `clippy` and `rustfmt`
- A C compiler, CMake, NASM, and pkg-config (the JPEG codec builds libjpeg-turbo from source)
- Node.js 22 and npm, for the admin UI
- `shellcheck`, if you change a shell script
- OpenSSL, for the local certificate in the README quick start

## Checks

From a checkout:

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

A short local benchmark, which serves its own fixtures and writes a result under `benchmarks/results/`:

```bash
bash benchmarks/run.sh --serve-fixtures --iterations 1 --concurrency 1 --sizes 100
```

That result is local. Do not commit it, and do not put numbers from it in the README unless they were measured for a published run.

## Tests

Add a test for every behaviour change. Server tests live next to the code they cover, and the in-process resize tests are in `server/src/integration_tests.rs`. Run a single test with `cargo test --locked <name>` from `server/`.

## Commit messages

Use a short prefix and an imperative summary, for example `fix(security): bound resize dimensions`, `docs: describe the URL format`, `chore: fix clippy warnings`, `test: cover unknown hosts`, `ci: add the GitHub Actions workflow`.

## Conduct and security

The community standard is [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md). Report vulnerabilities the way [SECURITY.md](SECURITY.md) describes, not in a public issue.
