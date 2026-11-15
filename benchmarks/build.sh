#!/usr/bin/env bash
#
# Speedwholes Build Helper
#
# Builds the server release binary via cargo. Intended to be called
# before run.sh, or by compare.sh as part of a multi-revision benchmark.
#
# Usage:
#   ./benchmarks/build.sh --source-dir DIR
#
# Options:
#   --source-dir DIR     Path to a repo checkout containing server/ (required)
#   --force              Rebuild even if binary already exists
#   --help               Show this help
#
set -euo pipefail

SOURCE_DIR=""
FORCE=false

# ---------------------------------------------------------------------------
# Parse CLI arguments
# ---------------------------------------------------------------------------
while [[ $# -gt 0 ]]; do
    case "$1" in
        --source-dir)  SOURCE_DIR="$2"; shift 2 ;;
        --force)       FORCE=true;      shift   ;;
        --help|-h)
            sed -n '/^# Speedwholes/,/^[^#]/{ /^#/{ s/^# \?//; p } }' "$0"
            exit 0 ;;
        *) echo "Unknown option: $1" >&2; exit 1 ;;
    esac
done

if [[ -z "$SOURCE_DIR" ]]; then
    echo "--source-dir is required" >&2
    exit 1
fi

SOURCE_DIR="$(cd "$SOURCE_DIR" && pwd)"
SERVER_DIR="$SOURCE_DIR/server"

if [[ ! -d "$SERVER_DIR" ]]; then
    echo "No server/ directory found in $SOURCE_DIR" >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# Dependency check
# ---------------------------------------------------------------------------
command -v cargo &>/dev/null || { echo "Missing required tool: cargo" >&2; exit 1; }

# ---------------------------------------------------------------------------
# Skip if binary already exists
# ---------------------------------------------------------------------------
BINARY="$SERVER_DIR/target/release/speedwholes"
if [[ "$FORCE" == "false" && -f "$BINARY" ]]; then
    echo "[build] Binary already exists at $BINARY -- skipping build (use --force to rebuild)"
    exit 0
fi

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------
echo "[build] Building release binary in $SERVER_DIR ..."
BUILD_START=$(date +%s)

(cd "$SERVER_DIR" && cargo build --release)

BUILD_END=$(date +%s)
BUILD_SECS=$(( BUILD_END - BUILD_START ))
echo "[build] Build complete (${BUILD_SECS}s)"
