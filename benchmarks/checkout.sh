#!/usr/bin/env bash
#
# Speedwholes Checkout Helper
#
# Clones (or updates) a repo into --clone-dir and checks out
# the requested revision. Prints the resolved short commit hash to stdout.
#
# Usage:
#   ./benchmarks/checkout.sh --repo PATH|URL --clone-dir DIR --revision REF
#
# Options:
#   --repo PATH|URL      Local repo path or remote URL to clone from (required)
#   --clone-dir DIR      Directory to clone into (required)
#   --revision REF       Branch, tag, or commit SHA to check out (required)
#   --help               Show this help
#
set -euo pipefail

REPO_URL=""
CLONE_DIR=""
REVISION=""

# ---------------------------------------------------------------------------
# Parse CLI arguments
# ---------------------------------------------------------------------------
while [[ $# -gt 0 ]]; do
    case "$1" in
        --repo)        REPO_URL="$2";  shift 2 ;;
        --clone-dir)   CLONE_DIR="$2"; shift 2 ;;
        --revision)    REVISION="$2";  shift 2 ;;
        --help|-h)
            sed -n '/^# Speedwholes/,/^[^#]/{ /^#/{ s/^# \?//; p } }' "$0"
            exit 0 ;;
        *) echo "Unknown option: $1" >&2; exit 1 ;;
    esac
done

if [[ -z "$REPO_URL" ]]; then
    echo "--repo is required" >&2; exit 1
fi
if [[ -z "$CLONE_DIR" ]]; then
    echo "--clone-dir is required" >&2; exit 1
fi
if [[ -z "$REVISION" ]]; then
    echo "--revision is required" >&2; exit 1
fi

# ---------------------------------------------------------------------------
# Dependency check
# ---------------------------------------------------------------------------
command -v git &>/dev/null || { echo "Missing required tool: git" >&2; exit 1; }

# ---------------------------------------------------------------------------
# Allow local repos regardless of ownership (container / CI environments).
# Uses per-command env var so we never touch the global gitconfig.
# ---------------------------------------------------------------------------
export GIT_CONFIG_COUNT=1
export GIT_CONFIG_KEY_0=safe.directory
export GIT_CONFIG_VALUE_0="*"

# ---------------------------------------------------------------------------
# Clone or fetch
# ---------------------------------------------------------------------------
if [[ -d "$CLONE_DIR/.git" ]]; then
    echo "[checkout] Updating existing clone in $CLONE_DIR ..." >&2
    (cd "$CLONE_DIR" && git fetch --all --quiet)
else
    echo "[checkout] Cloning $REPO_URL into $CLONE_DIR ..." >&2
    mkdir -p "$(dirname "$CLONE_DIR")"
    git clone --no-hardlinks "$REPO_URL" "$CLONE_DIR"
fi

# ---------------------------------------------------------------------------
# Checkout the requested revision
# ---------------------------------------------------------------------------
(cd "$CLONE_DIR" && git checkout --force "$REVISION" --quiet)
echo "[checkout] Checked out: $REVISION" >&2

# ---------------------------------------------------------------------------
# Print resolved short commit hash to stdout (for callers to capture)
# ---------------------------------------------------------------------------
(cd "$CLONE_DIR" && git rev-parse --short HEAD)
