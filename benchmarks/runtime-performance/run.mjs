import { cpus, platform, release, arch, totalmem } from "node:os";
import { existsSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(scriptDir, "..", "..");
const args = process.argv.slice(2);

function option(name, fallback) {
  const index = args.indexOf(name);
  if (index === -1) return fallback;
  const value = args[index + 1];
  if (!value) throw new Error(name + " requires a value");
  return value;
}

const runs = Number(option("--runs", "30"));
const iterations = Number(option("--iterations", "30"));
if (!Number.isInteger(runs) || runs < 1 || runs > 1000) throw new Error("--runs must be 1..1000");
if (!Number.isInteger(iterations) || iterations < 1 || iterations > 10000) {
  throw new Error("--iterations must be 1..10000");
}
const outputPath = resolve(repoRoot, option("--out", "reports/runtime-performance/latest.json"));
const binary = resolve(
  repoRoot,
  "target",
  "debug",
  process.platform === "win32" ? "kerna.exe" : "kerna",
);

function run(command, commandArgs, options = {}) {
  const result = spawnSync(command, commandArgs, {
    cwd: repoRoot,
    encoding: "utf8",
    timeout: 120_000,
    ...options,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error((result.stderr || result.stdout || "command failed").trim());
  }
  return result.stdout;
}

function percentile(values, percentileValue) {
  const sorted = [...values].sort((a, b) => a - b);
  const index = Math.min(sorted.length - 1, Math.max(0, Math.ceil(sorted.length * percentileValue) - 1));
  return Number(sorted[index].toFixed(3));
}

function summary(values) {
  return {
    samples: values.length,
    minMs: Number(Math.min(...values).toFixed(3)),
    p50Ms: percentile(values, 0.5),
    p95Ms: percentile(values, 0.95),
    p99Ms: percentile(values, 0.99),
    maxMs: Number(Math.max(...values).toFixed(3)),
  };
}

run("cargo", ["build", "--manifest-path", "kernel/Cargo.toml"]);
if (!existsSync(binary)) throw new Error("Expected Kerna binary at " + binary);

const initializationMs = [];
const discoveryMs = [];
const toolCallMs = [];
for (let runIndex = 0; runIndex < runs; runIndex += 1) {
  const output = run(binary, ["mcp", "benchmark-echo", "--iterations", String(iterations)]);
  const result = JSON.parse(output.trim());
  initializationMs.push(result.initializationMs);
  discoveryMs.push(result.toolDiscoveryMs);
  toolCallMs.push(...result.toolCallMs);
}

const gitRevision = run("git", ["rev-parse", "HEAD"]).trim();
const report = {
  benchmark: "Kerna MCP stdio performance",
  version: 1,
  deterministicFixture: "built-in MockMCP echo",
  sourceRevision: gitRevision,
  executedAt: new Date().toISOString(),
  environment: {
    operatingSystem: platform() + " " + release(),
    architecture: arch(),
    cpuModel: cpus()[0]?.model ?? "unknown",
    cpuCount: cpus().length,
    totalMemoryBytes: totalmem(),
  },
  configuration: {
    processRuns: runs,
    toolCallsPerRun: iterations,
    totalToolCalls: toolCallMs.length,
    scope: "Kerna stdio MCP client process isolation, initialization, tool discovery, and echo calls. Excludes scheduler, SQLite, provider, network, and model latency.",
  },
  metrics: {
    initialization: summary(initializationMs),
    toolDiscovery: summary(discoveryMs),
    toolCall: summary(toolCallMs),
  },
};

mkdirSync(dirname(outputPath), { recursive: true });
rmSync(outputPath, { force: true });
writeFileSync(outputPath, JSON.stringify(report, null, 2) + "\n");
console.log(
  "[runtime-performance] " + runs + " process runs and " + toolCallMs.length
    + " tool calls complete. Report: " + outputPath,
);
console.log(JSON.stringify(report.metrics, null, 2));
