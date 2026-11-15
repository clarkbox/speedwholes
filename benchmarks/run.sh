#!/usr/bin/env bash
#
# Speedwholes Resize Benchmark
#
# Starts a pre-built server binary and hands off to bench.mjs for the
# actual benchmarking. The binary must already exist (use build.sh first).
#
# Bring your own benchmarks/sources.json (gitignored). If that file is
# absent, this script uses benchmarks/sources.example.json, whose URLs point
# at the local fixture server started by --serve-fixtures.
#
# Usage:
#   ./benchmarks/run.sh [OPTIONS]
#
# Options:
#   --source-dir DIR     Use an external repo checkout instead of local source
#   --sources FILE       JSON file with image URLs
#                        (default: sources.json if present, else sources.example.json)
#   --serve-fixtures     Serve server/tests/fixtures on 127.0.0.1 (default: on)
#   --no-serve-fixtures  Do not start the local fixture server
#   --iterations N       Requests per source/size combo (default: 3)
#   --concurrency N      Parallel requests (default: 4)
#   --port PORT          Server port (default: random high port)
#   --sizes "100,200,…"  Override default size list
#   --output FILE        Override output file path for results JSON
#   --help               Show this help
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
SERVER_DIR="$REPO_ROOT/server"

# Defaults
SOURCE_DIR=""
SOURCES=""
SERVE_FIXTURES=1
FIXTURE_PORT="${FIXTURE_PORT:-8099}"
ITERATIONS=3
CONCURRENCY=4
PORT=""
SIZES="100,200,250,300,500,600,1200,3000"
OUTPUT=""

# ---------------------------------------------------------------------------
# Parse CLI arguments
# ---------------------------------------------------------------------------
while [[ $# -gt 0 ]]; do
    case "$1" in
        --source-dir)  SOURCE_DIR="$2";  shift 2 ;;
        --sources)     SOURCES="$2";     shift 2 ;;
        --serve-fixtures) SERVE_FIXTURES=1; shift ;;
        --no-serve-fixtures) SERVE_FIXTURES=0; shift ;;
        --iterations)  ITERATIONS="$2";  shift 2 ;;
        --concurrency) CONCURRENCY="$2"; shift 2 ;;
        --port)        PORT="$2";        shift 2 ;;
        --sizes)       SIZES="$2";       shift 2 ;;
        --output)      OUTPUT="$2";      shift 2 ;;
        --help|-h)
            sed -n '/^# Speedwholes/,/^[^#]/{ /^#/{ s/^# \?//; p } }' "$0"
            exit 0 ;;
        *) echo "Unknown option: $1" >&2; exit 1 ;;
    esac
done

# ---------------------------------------------------------------------------
# Resolve --source-dir (override SERVER_DIR if provided)
# ---------------------------------------------------------------------------
if [[ -n "$SOURCE_DIR" ]]; then
    SOURCE_DIR="$(cd "$SOURCE_DIR" && pwd)"
    SERVER_DIR="$SOURCE_DIR/server"
    [[ -d "$SERVER_DIR" ]] || { echo "No server/ directory found in $SOURCE_DIR" >&2; exit 1; }
    echo "[bench] Using external source dir: $SOURCE_DIR"
else
    echo "[bench] Using local source dir: $REPO_ROOT"
fi

[[ -z "$PORT" ]] && PORT=$(( RANDOM % 10000 + 20000 ))

if [[ -z "$SOURCES" ]]; then
    if [[ -f "$SCRIPT_DIR/sources.json" ]]; then
        SOURCES="$SCRIPT_DIR/sources.json"
    else
        SOURCES="$SCRIPT_DIR/sources.example.json"
    fi
fi
echo "[bench] Sources: $SOURCES"

# ---------------------------------------------------------------------------
# Dependency checks
# ---------------------------------------------------------------------------
for cmd in node curl openssl; do
    command -v "$cmd" &>/dev/null || { echo "Missing required tool: $cmd" >&2; exit 1; }
done
if [[ "$SERVE_FIXTURES" == "1" ]]; then
    command -v python3 &>/dev/null || { echo "Missing required tool: python3" >&2; exit 1; }
fi

# ---------------------------------------------------------------------------
# Temp dir + cleanup
# ---------------------------------------------------------------------------
TMPDIR_BENCH=$(mktemp -d "${TMPDIR:-/tmp}/speedwholes-bench.XXXXXX")
SERVER_PID=""
FIXTURE_PID=""
ADMIN_TOKEN="bench_admin_token_$$"

cleanup() {
    echo "[bench] Cleaning up..."
    [[ -n "$SERVER_PID" ]] && kill "$SERVER_PID" 2>/dev/null && wait "$SERVER_PID" 2>/dev/null || true
    [[ -n "$FIXTURE_PID" ]] && kill "$FIXTURE_PID" 2>/dev/null && wait "$FIXTURE_PID" 2>/dev/null || true
    [[ -d "$TMPDIR_BENCH" ]] && rm -rf "$TMPDIR_BENCH"
}
trap cleanup EXIT

if [[ "$SERVE_FIXTURES" == "1" ]]; then
    FIXTURE_DIR="${SOURCE_DIR:-$REPO_ROOT}/server/tests/fixtures"
    echo "[bench] Serving fixtures from $FIXTURE_DIR on 127.0.0.1:$FIXTURE_PORT"
    python3 -m http.server "$FIXTURE_PORT" --bind 127.0.0.1 -d "$FIXTURE_DIR" \
        > "$TMPDIR_BENCH/fixtures.log" 2>&1 &
    FIXTURE_PID=$!
    SOURCES_RUNTIME="$TMPDIR_BENCH/sources.json"
    sed -e "s|127.0.0.1:8099|127.0.0.1:${FIXTURE_PORT}|g" \
        -e "s|\${FIXTURE_PORT:-8099}|${FIXTURE_PORT}|g" \
        -e "s|\${FIXTURE_PORT}|${FIXTURE_PORT}|g" \
        "$SOURCES" > "$SOURCES_RUNTIME"
    SOURCES="$SOURCES_RUNTIME"
fi

# ---------------------------------------------------------------------------
# Verify binary exists (use build.sh to build first)
# ---------------------------------------------------------------------------
BINARY="$SERVER_DIR/target/release/speedwholes"
if [[ ! -f "$BINARY" ]]; then
    echo "No release binary found at $BINARY" >&2
    echo "Run build.sh first:  ./benchmarks/build.sh --source-dir ${SOURCE_DIR:-$REPO_ROOT}" >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# TLS certs
# ---------------------------------------------------------------------------
CERT_DIR="$TMPDIR_BENCH/certs"
mkdir -p "$CERT_DIR"
openssl req -x509 -newkey rsa:2048 -nodes \
    -keyout "$CERT_DIR/key.pem" -out "$CERT_DIR/cert.pem" \
    -days 1 -subj "/CN=localhost" \
    -addext "subjectAltName=DNS:localhost,IP:127.0.0.1" 2>/dev/null

# ---------------------------------------------------------------------------
# Server config
# ---------------------------------------------------------------------------
CONFIG_DIR="$TMPDIR_BENCH/config"
mkdir -p "$CONFIG_DIR"
cat > "$CONFIG_DIR/speedwholes.config.json" <<JSON
{
  "port": $PORT,
  "cache_path": "$TMPDIR_BENCH/cache",
  "max_width": 4096,
  "max_height": 4096,
  "timeout": 60,
  "max_source_pixels": 100000000,
  "allowed_sources": [],
  "allow_all_sources": true,
  "allow_private_networks": true,
  "unknown_host_policy": "global",
  "admin_token": "$ADMIN_TOKEN",
  "tls_cert": "$CERT_DIR/cert.pem",
  "tls_key": "$CERT_DIR/key.pem"
}
JSON

# ---------------------------------------------------------------------------
# Start server
# ---------------------------------------------------------------------------
echo "[bench] Starting server on port $PORT..."
"$SERVER_DIR/target/release/speedwholes" \
    --port "$PORT" --config-dir "$CONFIG_DIR" --log-format text \
    > "$TMPDIR_BENCH/server.log" 2>&1 &
SERVER_PID=$!

# Health check
for i in $(seq 1 30); do
    curl -ksS --max-time 2 "https://localhost:$PORT/health" &>/dev/null && break
    kill -0 "$SERVER_PID" 2>/dev/null || { echo "Server died. Log:"; tail -20 "$TMPDIR_BENCH/server.log"; exit 1; }
    sleep 1
done
echo "[bench] Server ready (PID $SERVER_PID)"

# ---------------------------------------------------------------------------
# Git commit (auto-detect from source dir)
# ---------------------------------------------------------------------------
GIT_SOURCE="${SOURCE_DIR:-$REPO_ROOT}"
GIT_COMMIT=$(cd "$GIT_SOURCE" && git rev-parse --short HEAD 2>/dev/null || echo "unknown")

# ---------------------------------------------------------------------------
# Run the Node.js benchmark
# ---------------------------------------------------------------------------
if [[ -z "$OUTPUT" ]]; then
    EPOCH=$(date +%s)
    OUTPUT="$SCRIPT_DIR/results/bench_${EPOCH}.json"
fi

node --no-warnings "$SCRIPT_DIR/bench.mjs" \
    --base-url "https://localhost:$PORT" \
    --sources "$SOURCES" \
    --iterations "$ITERATIONS" \
    --concurrency "$CONCURRENCY" \
    --sizes "$SIZES" \
    --admin-token "$ADMIN_TOKEN" \
    --output "$OUTPUT" \
    --git-commit "$GIT_COMMIT" \
    ${SOURCE_DIR:+--source-dir "$SOURCE_DIR"}
