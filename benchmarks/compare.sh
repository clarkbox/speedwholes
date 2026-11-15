#!/usr/bin/env bash
#
# Speedwholes Multi-Revision Benchmark
#
# Checks out, builds, and benchmarks multiple git revisions so you can
# compare performance across branches or commits.
#
# Usage:
#   ./benchmarks/compare.sh --working-dir DIR --revisions REV1,REV2,... [OPTIONS]
#
# Required:
#   --working-dir DIR          Directory for clones, builds, and results
#   --revisions REV1,REV2,...  Comma-separated branches, tags, or commit SHAs
#
# Options:
#   --repo PATH|URL      Local repo path or remote URL (default: local repo root)
#   --sources FILE       JSON file with image URLs
#                        (default: sources.json if present, else sources.example.json)
#   --iterations N       Requests per source/size combo (default: 3)
#   --concurrency N      Parallel requests (default: 4)
#   --sizes "100,200,…"  Override default size list
#   --help               Show this help
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Defaults
WORKING_DIR=""
REVISIONS=""
REPO_URL=""
SOURCES=""
SOURCES_SET=0
ITERATIONS=""
CONCURRENCY=""
SIZES=""

# ---------------------------------------------------------------------------
# Parse CLI arguments
# ---------------------------------------------------------------------------
while [[ $# -gt 0 ]]; do
    case "$1" in
        --working-dir) WORKING_DIR="$2"; shift 2 ;;
        --revisions)   REVISIONS="$2";   shift 2 ;;
        --repo)        REPO_URL="$2";    shift 2 ;;
        --sources)     SOURCES="$2"; SOURCES_SET=1; shift 2 ;;
        --iterations)  ITERATIONS="$2";  shift 2 ;;
        --concurrency) CONCURRENCY="$2"; shift 2 ;;
        --sizes)       SIZES="$2";       shift 2 ;;
        --help|-h)
            sed -n '/^# Speedwholes/,/^[^#]/{ /^#/{ s/^# \?//; p } }' "$0"
            exit 0 ;;
        *) echo "Unknown option: $1" >&2; exit 1 ;;
    esac
done

# ---------------------------------------------------------------------------
# Validate required args
# ---------------------------------------------------------------------------
if [[ -z "$WORKING_DIR" ]]; then
    echo "--working-dir is required" >&2; exit 1
fi
if [[ -z "$REVISIONS" ]]; then
    echo "--revisions is required (comma-separated list)" >&2; exit 1
fi

if [[ "$SOURCES_SET" -eq 0 ]]; then
    if [[ -f "$SCRIPT_DIR/sources.json" ]]; then
        SOURCES="$SCRIPT_DIR/sources.json"
    else
        SOURCES="$SCRIPT_DIR/sources.example.json"
    fi
fi

# Default repo: use the local repo on disk (fast, no auth needed).
# Override with --repo to point at a remote URL or different local path.
if [[ -z "$REPO_URL" ]]; then
    REPO_URL="$REPO_ROOT"
fi

# Split revisions into array
IFS=',' read -ra REV_LIST <<< "$REVISIONS"

if [[ ${#REV_LIST[@]} -eq 0 ]]; then
    echo "No revisions specified" >&2; exit 1
fi

# Ensure working dir exists
mkdir -p "$WORKING_DIR"
WORKING_DIR="$(cd "$WORKING_DIR" && pwd)"

# Results directory inside working dir
RESULTS_DIR="$WORKING_DIR/results"
mkdir -p "$RESULTS_DIR"

# ---------------------------------------------------------------------------
# Build passthrough args for run.sh
# ---------------------------------------------------------------------------
RUN_ARGS=()
RUN_ARGS+=(--sources "$SOURCES")
[[ -n "$ITERATIONS" ]]  && RUN_ARGS+=(--iterations "$ITERATIONS")
[[ -n "$CONCURRENCY" ]] && RUN_ARGS+=(--concurrency "$CONCURRENCY")
[[ -n "$SIZES" ]]       && RUN_ARGS+=(--sizes "$SIZES")

# ---------------------------------------------------------------------------
# Banner
# ---------------------------------------------------------------------------
echo ""
echo "======================================================================="
echo "  Speedwholes Multi-Revision Benchmark"
echo "======================================================================="
echo "  Repo:        $REPO_URL"
echo "  Working dir: $WORKING_DIR"
echo "  Revisions:   ${REV_LIST[*]}"
echo "  Results dir: $RESULTS_DIR"
echo "======================================================================="
echo ""

# ---------------------------------------------------------------------------
# Track results for summary
# ---------------------------------------------------------------------------
declare -a RESULT_REVS=()
declare -a RESULT_COMMITS=()
declare -a RESULT_FILES=()
declare -a RESULT_STATUS=()

# ---------------------------------------------------------------------------
# Process each revision
# ---------------------------------------------------------------------------
TOTAL=${#REV_LIST[@]}
IDX=0

for REV in "${REV_LIST[@]}"; do
    IDX=$(( IDX + 1 ))
    echo ""
    echo "-----------------------------------------------------------------------"
    echo "  [$IDX/$TOTAL] Revision: $REV"
    echo "-----------------------------------------------------------------------"
    echo ""

    # Sanitise revision name for use in filenames (replace / with -)
    REV_LABEL="${REV//\//-}"
    EPOCH=$(date +%s)
    OUTPUT_FILE="$RESULTS_DIR/bench_${REV_LABEL}_${EPOCH}.json"

    # Per-revision clone directory
    CLONE_DIR="$WORKING_DIR/repo-${REV_LABEL}"

    # -- Checkout --
    COMMIT_HASH=$("$SCRIPT_DIR/checkout.sh" \
        --repo "$REPO_URL" \
        --clone-dir "$CLONE_DIR" \
        --revision "$REV") || {
        echo "[compare] FAILED to checkout $REV" >&2
        RESULT_REVS+=("$REV")
        RESULT_COMMITS+=("n/a")
        RESULT_FILES+=("n/a")
        RESULT_STATUS+=("CHECKOUT FAILED")
        continue
    }
    echo "[compare] Resolved $REV -> $COMMIT_HASH"

    # -- Build --
    "$SCRIPT_DIR/build.sh" --source-dir "$CLONE_DIR" || {
        echo "[compare] FAILED to build $REV ($COMMIT_HASH)" >&2
        RESULT_REVS+=("$REV")
        RESULT_COMMITS+=("$COMMIT_HASH")
        RESULT_FILES+=("n/a")
        RESULT_STATUS+=("BUILD FAILED")
        continue
    }

    # -- Run benchmark --
    "$SCRIPT_DIR/run.sh" \
        --source-dir "$CLONE_DIR" \
        --output "$OUTPUT_FILE" \
        "${RUN_ARGS[@]}" || {
        echo "[compare] FAILED to benchmark $REV ($COMMIT_HASH)" >&2
        RESULT_REVS+=("$REV")
        RESULT_COMMITS+=("$COMMIT_HASH")
        RESULT_FILES+=("$OUTPUT_FILE")
        RESULT_STATUS+=("BENCH FAILED")
        continue
    }

    RESULT_REVS+=("$REV")
    RESULT_COMMITS+=("$COMMIT_HASH")
    RESULT_FILES+=("$OUTPUT_FILE")
    RESULT_STATUS+=("OK")
done

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo ""
echo "======================================================================="
echo "  COMPARISON COMPLETE"
echo "======================================================================="
printf "  %-30s %-10s %-10s %s\n" "REVISION" "COMMIT" "STATUS" "RESULTS FILE"
printf "  %-30s %-10s %-10s %s\n" "--------" "------" "------" "------------"

for i in "${!RESULT_REVS[@]}"; do
    printf "  %-30s %-10s %-10s %s\n" \
        "${RESULT_REVS[$i]}" \
        "${RESULT_COMMITS[$i]}" \
        "${RESULT_STATUS[$i]}" \
        "${RESULT_FILES[$i]}"
done

echo "======================================================================="
echo ""

# ---------------------------------------------------------------------------
# Comparison table: avg resize time per size across all sources
# ---------------------------------------------------------------------------
COMPARE_JSON="["
FIRST=true
for i in "${!RESULT_REVS[@]}"; do
    if [[ "${RESULT_STATUS[$i]}" == "OK" ]]; then
        $FIRST || COMPARE_JSON+=","
        FIRST=false
        COMPARE_JSON+="{\"rev\":\"${RESULT_REVS[$i]}\",\"commit\":\"${RESULT_COMMITS[$i]}\",\"file\":\"${RESULT_FILES[$i]}\"}"
    fi
done
COMPARE_JSON+="]"

# Only print comparison table if we have at least one successful result
OK_COUNT=0
for s in "${RESULT_STATUS[@]}"; do [[ "$s" == "OK" ]] && OK_COUNT=$(( OK_COUNT + 1 )); done

if [[ "$OK_COUNT" -gt 0 ]]; then
    cat > "$WORKING_DIR/_compare_table.cjs" <<'NODESCRIPT'
const { readFileSync } = require("fs");

const C = {
  cyan: (s) => `\x1b[36m${s}\x1b[0m`,
  green: (s) => `\x1b[32m${s}\x1b[0m`,
  bold: (s) => `\x1b[1m${s}\x1b[0m`,
};

const rgt = (s, w) => String(s).padStart(w);
const lft = (s, w) => String(s).padEnd(w);

const entries = JSON.parse(process.argv[2]);
if (entries.length === 0) process.exit(0);

// Load all result files
const loaded = entries.map((e) => {
  const data = JSON.parse(readFileSync(e.file, "utf-8"));
  return { ...e, data };
});

// Collect all sizes tested (union, sorted numerically)
const allSizes = [
  ...new Set(loaded.flatMap((l) => l.data.sizes_tested)),
].sort((a, b) => a - b);

// For each entry, compute avg resize_ms per size across all sources
const rows = loaded.map((l) => {
  const avgs = {};
  for (const size of allSizes) {
    const matching = l.data.results.filter((r) => r.width === size);
    const times = matching
      .flatMap((e) => e.iterations.map((it) => it.resize_ms))
      .filter((v) => v !== null);
    avgs[size] =
      times.length > 0
        ? +(times.reduce((a, b) => a + b, 0) / times.length).toFixed(2)
        : null;
  }
  return { rev: l.rev, commit: l.commit, avgs };
});

// Column widths
const revW = Math.max(16, ...rows.map((r) => r.rev.length)) + 2;
const commitW = 10;
const sizeW = 10;

// Header
console.log();
console.log(
  C.bold(
    "═══════════════════════════════════════════════════════════════════════"
  )
);
console.log(
  C.bold("  COMPARISON: Average Resize Time by Size (ms)")
);
console.log(
  C.bold(
    "═══════════════════════════════════════════════════════════════════════"
  )
);
console.log();

const sizeCols = allSizes.map((s) => rgt(s, sizeW)).join("");
console.log(
  C.bold(
    `  ${lft("REVISION", revW)}${lft("COMMIT", commitW)}${sizeCols}`
  )
);

const sizeSep = allSizes.map(() => "─".repeat(sizeW)).join("");
console.log(
  `  ${"─".repeat(revW)}${"─".repeat(commitW)}${sizeSep}`
);

// Data rows
for (const row of rows) {
  const vals = allSizes
    .map((s) => {
      const v = row.avgs[s];
      return rgt(v !== null ? v : "n/a", sizeW);
    })
    .join("");
  console.log(
    `  ${lft(row.rev, revW)}${lft(row.commit, commitW)}${vals}`
  );
}

console.log();
console.log(
  C.bold(
    "═══════════════════════════════════════════════════════════════════════"
  )
);
console.log();
NODESCRIPT

    node --no-warnings "$WORKING_DIR/_compare_table.cjs" "$COMPARE_JSON"
    rm -f "$WORKING_DIR/_compare_table.cjs"
fi

# Exit non-zero if any revision failed
for status in "${RESULT_STATUS[@]}"; do
    if [[ "$status" != "OK" ]]; then
        exit 1
    fi
done
