# Development Guide

## Prerequisites

- Rust 1.70+ (`rustup` recommended)
- Node.js 18+ (for frontend)
- tmux (for `startLocalDev.sh`)
- C compiler (gcc or clang)
- CMake and NASM -- required to build libjpeg-turbo from source (used by the `turbojpeg` crate for SIMD-accelerated JPEG processing)

### Installing build dependencies

```bash
# RHEL / CentOS / Fedora
sudo dnf install cmake nasm gcc

# Ubuntu / Debian
sudo apt install cmake nasm build-essential

# macOS (Homebrew)
brew install cmake nasm
```

The JPEG codec builds libjpeg-turbo from source. A system libjpeg package is not used.

## Quick Start

```bash
# Start both services in tmux
./startLocalDev.sh

# Or with custom log location
./startLocalDev.sh --logPath ./logs
```

Access points:
- Frontend UI: http://127.0.0.1:8081
- Backend API: https://localhost:3000

The script writes `dev-config/` (gitignored) and passes `--config-dir`. The API certificate is self-signed.

## Manual Start (Two Terminals)

The [root README](../README.md) quick start is the path that creates a config, a certificate, and a working resize URL. The short form, once `dev-config/` exists:

```bash
# Terminal 1 - Backend
cd server
cargo run -- --config-dir ../dev-config --log-format text

# Terminal 2 - Frontend
cd www
npm install
npm run dev
```

## Configuration

### Backend (server/)

All configuration is file-based via `server/speedwholes.config.json`:

```json
{
  "port": 3000,
  "cache_path": "./cache",
  "admin_token": "your-secret-token",
  "max_width": 4096,
  "max_height": 4096,
  "timeout": 30
}
```

CLI arguments (`--port`, `--config-dir`, `--cache-path`) override config file values.

Set `RUST_LOG=info` for log level (this is the only env var used).

### Frontend (www/)

Edit `www/public/config.json` to set the backend API URL:

```json
{
  "apiUrl": "https://localhost:3000"
}
```

## Common Commands

```bash
# Build backend
cd server && cargo build --release

# Build frontend
cd www && npm run build

# Test
cd server && cargo test

# Format/Lint
cargo fmt
cargo clippy

# Watch mode (auto-reload)
cargo watch -x run
```

## Tmux Controls

When using `startLocalDev.sh`:

- Switch panes: `Ctrl+b` + arrow keys
- Detach: `Ctrl+b` + `d`
- Reattach: `tmux attach -t speedwholes`
- Kill session: `tmux kill-session -t speedwholes`

Logs saved to `/tmp/speedwholes-logs/` by default:
- `server.log` - Backend output
- `frontend.log` - Vite dev server output

## Troubleshooting

### Port in use

```bash
lsof -i :3000
kill $(lsof -t -i:3000)
```

### CORS errors

The server allows all origins by default. If you see CORS errors, ensure:
1. The backend is running and accessible
2. Check browser console for the actual error (may be network, not CORS)

### Clean rebuild

```bash
cd server && cargo clean && cargo build
cd www && rm -rf node_modules && npm install
```

## Project Structure

```
speedwholes/
├── server/           # Backend API (Rust + Axum)
├── www/              # Frontend (Vue 3 + Vite)
├── docs/             # Documentation
├── systemd-examples/ # Systemd service files
├── startLocalDev.sh  # Dev startup script
└── deploy.sh         # Deployment script
```

See component READMEs for details:
- [server/README.md](../server/README.md) - Backend API docs
- [www/README.md](../www/README.md) - Frontend dev guide
- [deployment.md](deployment.md) - Production deployment
