#!/usr/bin/env node
/**
 * Speedwholes Resize Benchmark
 *
 * Runs resize requests across all filter code paths, captures internal
 * resize timing from X-Speedwholes-Resize-Ms headers, and writes
 * structured results JSON.
 *
 * Zero external dependencies — uses Node 22 built-ins only.
 *
 * Usage (called by run.sh, or standalone):
 *   node benchmarks/bench.mjs \
 *     --base-url https://localhost:PORT \
 *     --sources benchmarks/sources.example.json \
 *     --iterations 3 \
 *     --concurrency 4 \
 *     --sizes 100,200,250,300,500,600,1200,3000 \
 *     --admin-token TOKEN \
 *     --output benchmarks/results/bench_EPOCH.json \
 *     --source-dir /path/to/external/checkout
 */

import { readFile, writeFile, mkdir } from "node:fs/promises";
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { execSync } from "node:child_process";

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));

function defaultSourcesPath() {
  const own = join(SCRIPT_DIR, "sources.json");
  if (existsSync(own)) return own;
  return join(SCRIPT_DIR, "sources.example.json");
}

// ---------------------------------------------------------------------------
// TLS: accept self-signed certs
// ---------------------------------------------------------------------------
process.env.NODE_TLS_REJECT_UNAUTHORIZED = "0";

// ---------------------------------------------------------------------------
// Colors
// ---------------------------------------------------------------------------
const C = {
  cyan: (s) => `\x1b[36m${s}\x1b[0m`,
  green: (s) => `\x1b[32m${s}\x1b[0m`,
  yellow: (s) => `\x1b[33m${s}\x1b[0m`,
  red: (s) => `\x1b[31m${s}\x1b[0m`,
  bold: (s) => `\x1b[1m${s}\x1b[0m`,
};

const log = (msg) => console.log(`${C.cyan("[bench]")} ${msg}`);
const ok = (msg) => console.log(`${C.green("[bench]")} ${msg}`);
const warn = (msg) => console.log(`${C.yellow("[bench]")} ${msg}`);

// ---------------------------------------------------------------------------
// CLI arg parsing
// ---------------------------------------------------------------------------
function parseArgs() {
  const args = process.argv.slice(2);
  const opts = {
    baseUrl: "",
    sources: defaultSourcesPath(),
    iterations: 3,
    concurrency: 4,
    sizes: [100, 200, 250, 300, 500, 600, 1200, 3000],
    adminToken: "",
    output: "",
    gitCommit: "unknown",
    sourceDir: "",
  };

  for (let i = 0; i < args.length; i++) {
    switch (args[i]) {
      case "--base-url":
        opts.baseUrl = args[++i];
        break;
      case "--sources":
        opts.sources = args[++i];
        break;
      case "--iterations":
        opts.iterations = parseInt(args[++i], 10);
        break;
      case "--concurrency":
        opts.concurrency = parseInt(args[++i], 10);
        break;
      case "--sizes":
        opts.sizes = args[++i].split(",").map(Number);
        break;
      case "--admin-token":
        opts.adminToken = args[++i];
        break;
      case "--output":
        opts.output = args[++i];
        break;
      case "--git-commit":
        opts.gitCommit = args[++i];
        break;
      case "--source-dir":
        opts.sourceDir = args[++i];
        break;
      default:
        console.error(`Unknown arg: ${args[i]}`);
        process.exit(1);
    }
  }

  if (!opts.baseUrl) {
    console.error("--base-url is required");
    process.exit(1);
  }
  if (!opts.output) {
    const epoch = Math.floor(Date.now() / 1000);
    opts.output = `benchmarks/results/bench_${epoch}.json`;
  }
  return opts;
}

// ---------------------------------------------------------------------------
// Filter classification (mirrors server/src/resize.rs thresholds)
// ---------------------------------------------------------------------------
const TINY_THRESHOLD = 201;
const FAST_THRESHOLD = 300;

function filterForWidth(w) {
  if (w <= TINY_THRESHOLD) return "Bilinear";
  if (w <= FAST_THRESHOLD) return "CatmullRom";
  return "Lanczos3";
}

// ---------------------------------------------------------------------------
// Concurrency pool — p-limit without dependencies
// ---------------------------------------------------------------------------
function makePool(limit) {
  let active = 0;
  const queue = [];
  const next = () => {
    while (queue.length > 0 && active < limit) {
      active++;
      queue.shift()();
    }
  };
  return (fn) =>
    new Promise((resolve, reject) => {
      queue.push(() =>
        fn()
          .then(resolve, reject)
          .finally(() => {
            active--;
            next();
          })
      );
      next();
    });
}

// ---------------------------------------------------------------------------
// Single resize request — returns timing data
// ---------------------------------------------------------------------------
async function benchOne(baseUrl, width, sourceUrl) {
  const url = `${baseUrl}/resize/w/${width}/h/0/nocache/1/u/${sourceUrl}`;
  const t0 = performance.now();

  try {
    const res = await fetch(url);
    const httpTotalMs = +(performance.now() - t0).toFixed(2);
    const body = await res.arrayBuffer();
    const outputBytes = body.byteLength;
    const httpCode = res.status;

    const resizeMs = res.headers.get("x-speedwholes-resize-ms");
    const fetchMs = res.headers.get("x-speedwholes-fetch-ms");
    const totalMs = res.headers.get("x-speedwholes-total-ms");

    return {
      resize_ms: resizeMs ? parseFloat(resizeMs) : null,
      fetch_ms: fetchMs ? parseFloat(fetchMs) : null,
      server_total_ms: totalMs ? parseFloat(totalMs) : null,
      http_total_ms: httpTotalMs,
      output_bytes: outputBytes,
      http_code: httpCode,
    };
  } catch (err) {
    const httpTotalMs = +(performance.now() - t0).toFixed(2);
    return {
      resize_ms: null,
      fetch_ms: null,
      server_total_ms: null,
      http_total_ms: httpTotalMs,
      output_bytes: 0,
      http_code: 0,
      error: err.message,
    };
  }
}

// ---------------------------------------------------------------------------
// Admin API helpers
// ---------------------------------------------------------------------------
async function adminLogin(baseUrl, token) {
  try {
    const res = await fetch(`${baseUrl}/config007/login`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ token }),
    });
    const data = await res.json();
    return data.session_token || null;
  } catch {
    return null;
  }
}

async function adminGet(baseUrl, sessionToken, path) {
  try {
    const res = await fetch(`${baseUrl}/config007/${path}`, {
      headers: { Authorization: `Bearer ${sessionToken}` },
    });
    return await res.json();
  } catch {
    return {};
  }
}

async function adminDelete(baseUrl, sessionToken, path) {
  try {
    await fetch(`${baseUrl}/config007/${path}`, {
      method: "DELETE",
      headers: { Authorization: `Bearer ${sessionToken}` },
    });
  } catch {
    // ignore
  }
}

// ---------------------------------------------------------------------------
// Warmup
// ---------------------------------------------------------------------------
async function warmup(baseUrl, sources, concurrency) {
  log(`Warming up source cache (${sources.length} source(s), concurrency=${concurrency})...`);

  const pool = makePool(concurrency);
  const warmupWidth = 300;

  // Pass 1: populate source + resize cache
  const pass1 = sources.map((src) =>
    pool(async () => {
      const url = `${baseUrl}/resize/w/${warmupWidth}/h/0/u/${src.url}`;
      try {
        const res = await fetch(url);
        await res.arrayBuffer(); // drain body
        return { name: src.name, status: res.status };
      } catch (err) {
        return { name: src.name, status: 0, error: err.message };
      }
    })
  );

  const results = await Promise.all(pass1);
  const okCount = results.filter((r) => r.status === 200).length;
  const failCount = results.length - okCount;
  log(`Warmup pass 1: ${okCount} OK, ${failCount} failed`);

  for (const r of results) {
    if (r.status !== 200) {
      warn(`  ${r.name}: HTTP ${r.status}${r.error ? ` (${r.error})` : ""}`);
    }
  }

  // Pass 2: hit with nocache to confirm source cache is warm
  log("Second warmup pass (nocache)...");
  const pass2 = sources.map((src) =>
    pool(async () => {
      const url = `${baseUrl}/resize/w/${warmupWidth}/h/0/nocache/1/u/${src.url}`;
      try {
        const res = await fetch(url);
        await res.arrayBuffer();
      } catch {
        // ignore
      }
    })
  );
  await Promise.all(pass2);
  ok("Warmup complete");
}

// ---------------------------------------------------------------------------
// Stats helpers
// ---------------------------------------------------------------------------
function computeStats(values) {
  const nums = values.filter((v) => v !== null && v !== undefined);
  if (nums.length === 0) return { avg: null, min: null, max: null };
  const sum = nums.reduce((a, b) => a + b, 0);
  return {
    avg: +(sum / nums.length).toFixed(2),
    min: +Math.min(...nums).toFixed(2),
    max: +Math.max(...nums).toFixed(2),
  };
}

// ---------------------------------------------------------------------------
// Main benchmark
// ---------------------------------------------------------------------------
async function runBenchmark(opts) {
  const { baseUrl, sources, iterations, concurrency, sizes, adminToken, output, gitCommit } = opts;

  // Admin login
  let sessionToken = null;
  if (adminToken) {
    log("Logging into admin API...");
    sessionToken = await adminLogin(baseUrl, adminToken);
    if (sessionToken) {
      ok("Admin session acquired");
    } else {
      warn("Could not obtain admin session (resize-stats unavailable)");
    }
  }

  // Warmup
  await warmup(baseUrl, sources, concurrency);

  // Reset stats
  if (sessionToken) {
    await adminDelete(baseUrl, sessionToken, "resize-stats");
  }

  // Prepare work items
  const totalRequests = sources.length * sizes.length * iterations;
  log(
    `Starting benchmark: ${sources.length} source(s) x ${sizes.length} size(s) x ${iterations} iter(s) = ${totalRequests} requests`
  );
  log(`Concurrency: ${concurrency} | Sizes: ${sizes.join(", ")}`);
  console.log();

  const pool = makePool(concurrency);
  const rawResults = []; // collect all results
  let completed = 0;
  let lastProgress = 0;

  // Build all jobs
  const jobs = [];
  for (const src of sources) {
    for (const width of sizes) {
      const filter = filterForWidth(width);
      for (let iter = 1; iter <= iterations; iter++) {
        jobs.push(
          pool(async () => {
            const result = await benchOne(baseUrl, width, src.url);
            rawResults.push({
              source: src.name,
              width,
              filter,
              iteration: iter,
              ...result,
            });

            completed++;
            // Progress every 25 completions or at the end
            if (completed - lastProgress >= 25 || completed === totalRequests) {
              lastProgress = completed;
              process.stdout.write(
                `\r  ${C.cyan(`Progress: ${completed} / ${totalRequests} requests`)}`
              );
            }
          })
        );
      }
    }
  }

  await Promise.all(jobs);
  process.stdout.write(
    `\r  ${C.green(`Completed: ${completed} / ${totalRequests} requests`)}\n`
  );
  console.log();

  // ---------------------------------------------------------------------------
  // Aggregate results per source+width
  // ---------------------------------------------------------------------------
  const failedRequests = rawResults.filter((r) => r.http_code !== 200).length;

  const aggregated = [];
  for (const src of sources) {
    for (const width of sizes) {
      const filter = filterForWidth(width);
      const matching = rawResults.filter(
        (r) => r.source === src.name && r.width === width
      );

      const resizeTimes = matching.map((r) => r.resize_ms);
      const stats = computeStats(resizeTimes);

      aggregated.push({
        source: src.name,
        width,
        filter,
        iterations: matching.map((r) => ({
          resize_ms: r.resize_ms,
          http_total_ms: r.http_total_ms,
          output_bytes: r.output_bytes,
        })),
        avg_resize_ms: stats.avg,
        min_resize_ms: stats.min,
        max_resize_ms: stats.max,
      });
    }
  }

  // ---------------------------------------------------------------------------
  // Print per-source summary (live, before writing file)
  // ---------------------------------------------------------------------------
  for (const src of sources) {
    console.log(C.bold(`Source: ${src.name}`));
    for (const width of sizes) {
      const filter = filterForWidth(width);
      const entry = aggregated.find(
        (a) => a.source === src.name && a.width === width
      );
      const label = `  w/${String(width).padEnd(5)} [${filter.padEnd(10)}]`;
      if (entry && entry.avg_resize_ms !== null) {
        console.log(
          `${label} ${C.green(`avg=${entry.avg_resize_ms}ms`)}  min=${entry.min_resize_ms}ms  max=${entry.max_resize_ms}ms`
        );
      } else {
        console.log(`${label} ${C.yellow("resize_ms unavailable or request failed")}`);
      }
    }
    console.log();
  }

  // ---------------------------------------------------------------------------
  // Fetch server aggregate stats
  // ---------------------------------------------------------------------------
  let serverStats = {};
  if (sessionToken) {
    serverStats = await adminGet(baseUrl, sessionToken, "resize-stats");
  }

  // ---------------------------------------------------------------------------
  // Build filter map
  // ---------------------------------------------------------------------------
  const filterMap = {};
  for (const w of sizes) {
    filterMap[w] = filterForWidth(w);
  }

  // ---------------------------------------------------------------------------
  // Write results JSON
  // ---------------------------------------------------------------------------
  const epoch = Math.floor(Date.now() / 1000);
  const sourceDir = opts.sourceDir;
  const resultsData = {
    timestamp: epoch,
    datetime: new Date().toISOString(),
    git_commit: gitCommit,
    source_dir: sourceDir || "(local)",
    build_profile: "release",
    iterations_per_combo: iterations,
    concurrency,
    total_requests: rawResults.length,
    failed_requests: failedRequests,
    sizes_tested: sizes,
    filter_map: filterMap,
    sources: sources.map((s) => s.name),
    results: aggregated,
    server_stats: serverStats,
  };

  await mkdir(dirname(output), { recursive: true });
  await writeFile(output, JSON.stringify(resultsData, null, 2) + "\n");
  ok(`Results written to ${output}`);

  // ---------------------------------------------------------------------------
  // Print summary table
  // ---------------------------------------------------------------------------
  console.log();
  console.log(C.bold("═══════════════════════════════════════════════════════════════════════"));
  console.log(C.bold("  BENCHMARK SUMMARY"));
  console.log(C.bold("═══════════════════════════════════════════════════════════════════════"));
  console.log(`  Commit:      ${gitCommit}`);
  if (opts.sourceDir) {
    console.log(`  Source dir:  ${opts.sourceDir}`);
  }
  console.log(`  Timestamp:   ${new Date().toISOString()}`);
  console.log(`  Iterations:  ${iterations} per combo (concurrency: ${concurrency})`);
  console.log(`  Requests:    ${rawResults.length} total, ${failedRequests} failed`);
  console.log();

  const hdr = (s, w) => s.padEnd(w);
  const rgt = (s, w) => String(s).padStart(w);
  console.log(
    C.bold(
      `  ${hdr("SOURCE", 20)} ${rgt("WIDTH", 6)}  ${hdr("FILTER", 10)}  ${rgt("AVG(ms)", 10)}  ${rgt("MIN(ms)", 10)}  ${rgt("MAX(ms)", 10)}`
    )
  );
  console.log(
    `  ${"─".repeat(20)} ${"─".repeat(6)}  ${"─".repeat(10)}  ${"─".repeat(10)}  ${"─".repeat(10)}  ${"─".repeat(10)}`
  );

  for (const entry of aggregated) {
    const avg = entry.avg_resize_ms !== null ? entry.avg_resize_ms : "n/a";
    const min = entry.min_resize_ms !== null ? entry.min_resize_ms : "n/a";
    const max = entry.max_resize_ms !== null ? entry.max_resize_ms : "n/a";
    console.log(
      `  ${hdr(entry.source, 20)} ${rgt(entry.width, 6)}  ${hdr(entry.filter, 10)}  ${rgt(avg, 10)}  ${rgt(min, 10)}  ${rgt(max, 10)}`
    );
  }

  // ---------------------------------------------------------------------------
  // Print per-size averages across all sources
  // ---------------------------------------------------------------------------
  console.log();
  console.log(C.bold("  AVERAGE RESIZE TIME PER SIZE (across all sources)"));
  console.log(
    `  ${"─".repeat(68)}`
  );
  console.log(
    C.bold(
      `  ${rgt("WIDTH", 6)}  ${hdr("FILTER", 10)}  ${rgt("AVG(ms)", 10)}  ${rgt("MIN(ms)", 10)}  ${rgt("MAX(ms)", 10)}  ${rgt("SOURCES", 8)}`
    )
  );
  console.log(
    `  ${"─".repeat(6)}  ${"─".repeat(10)}  ${"─".repeat(10)}  ${"─".repeat(10)}  ${"─".repeat(10)}  ${"─".repeat(8)}`
  );

  for (const width of sizes) {
    const filter = filterForWidth(width);
    const entriesForWidth = aggregated.filter(
      (a) => a.width === width && a.avg_resize_ms !== null
    );
    if (entriesForWidth.length === 0) {
      console.log(
        `  ${rgt(width, 6)}  ${hdr(filter, 10)}  ${rgt("n/a", 10)}  ${rgt("n/a", 10)}  ${rgt("n/a", 10)}  ${rgt(0, 8)}`
      );
      continue;
    }

    const allResizeTimes = entriesForWidth.flatMap((e) =>
      e.iterations.map((it) => it.resize_ms).filter((v) => v !== null)
    );
    const sizeStats = computeStats(allResizeTimes);

    console.log(
      `  ${rgt(width, 6)}  ${hdr(filter, 10)}  ${rgt(sizeStats.avg, 10)}  ${rgt(sizeStats.min, 10)}  ${rgt(sizeStats.max, 10)}  ${rgt(entriesForWidth.length, 8)}`
    );
  }

  console.log();
  console.log(C.bold("═══════════════════════════════════════════════════════════════════════"));
  console.log(`  Results: ${C.cyan(output)}`);
  console.log(C.bold("═══════════════════════════════════════════════════════════════════════"));
  console.log();
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------
async function main() {
  const opts = parseArgs();

  // Load sources
  const sourcesData = JSON.parse(await readFile(opts.sources, "utf-8"));
  opts.sources = sourcesData.sources;

  if (!opts.sources || opts.sources.length === 0) {
    console.error("No sources found in sources file");
    process.exit(1);
  }

  // Git commit fallback (auto-detect from source dir or cwd)
  if (opts.gitCommit === "unknown") {
    try {
      const gitDir = opts.sourceDir || ".";
      opts.gitCommit = execSync("git rev-parse --short HEAD", {
        encoding: "utf-8",
        cwd: gitDir,
      }).trim();
    } catch {
      // keep "unknown"
    }
  }

  console.log();
  console.log(C.bold("Speedwholes Resize Benchmark"));
  console.log("────────────────────────────────────────");
  console.log();

  if (opts.sourceDir) {
    log(`Source dir: ${opts.sourceDir}`);
  }
  log(`Sources: ${opts.sources.length} loaded`);
  log(`Sizes: ${opts.sizes.join(", ")}`);
  log(`Iterations: ${opts.iterations} | Concurrency: ${opts.concurrency}`);
  console.log();

  await runBenchmark(opts);
}

main().catch((err) => {
  console.error(C.red(`Fatal: ${err.message}`));
  process.exit(1);
});
