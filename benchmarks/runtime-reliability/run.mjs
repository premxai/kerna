import { arch, cpus, platform, release } from "node:os";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
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

const runs = Number(option("--runs", "120"));
const iterations = Number(option("--iterations", "20"));
if (!Number.isInteger(runs) || runs < 1 || runs > 10000) throw new Error("--runs must be 1..10000");
if (!Number.isInteger(iterations) || iterations < 1 || iterations > 10000) {
  throw new Error("--iterations must be 1..10000");
}
const outputPath = resolve(repoRoot, option("--out", "reports/runtime-reliability/latest.json"));
const binary = resolve(
  repoRoot,
  "target",
  "debug",
  process.platform === "win32" ? "kerna.exe" : "kerna",
);

function command(commandName, commandArgs, options = {}) {
  const result = spawnSync(commandName, commandArgs, {
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

command("cargo", ["build", "--manifest-path", "kernel/Cargo.toml"]);
if (!existsSync(binary)) throw new Error("Expected Kerna binary at " + binary);

const started = performance.now();
const failures = [];
let completedToolCalls = 0;
for (let index = 0; index < runs; index += 1) {
  const result = spawnSync(binary, ["mcp", "benchmark-echo", "--iterations", String(iterations)], {
    cwd: repoRoot,
    encoding: "utf8",
    timeout: 120_000,
  });
  if (result.error || result.status !== 0) {
    failures.push({
      run: index + 1,
      exitCode: result.status,
      error: result.error?.message ?? (result.stderr || result.stdout || "unknown failure").slice(-1000),
    });
    continue;
  }
  try {
    const output = JSON.parse(result.stdout.trim());
    if (!Array.isArray(output.toolCallMs) || output.toolCallMs.length !== iterations) {
      throw new Error("unexpected tool-call count");
    }
    completedToolCalls += output.toolCallMs.length;
  } catch (error) {
    failures.push({ run: index + 1, exitCode: result.status, error: String(error) });
  }
}

const gitRevision = command("git", ["rev-parse", "HEAD"]).trim();
const report = {
  benchmark: "Kerna MCP stdio restart soak",
  version: 1,
  sourceRevision: gitRevision,
  executedAt: new Date().toISOString(),
  environment: {
    operatingSystem: platform() + " " + release(),
    architecture: arch(),
    cpuModel: cpus()[0]?.model ?? "unknown",
  },
  configuration: {
    processRuns: runs,
    toolCallsPerRun: iterations,
    expectedToolCalls: runs * iterations,
    scope: "Repeated Kerna process startup, built-in MockMCP child-process spawn, initialize, discovery, echo calls, and clean process exit. Excludes scheduler, provider, network, and model behavior.",
  },
  result: {
    successfulRuns: runs - failures.length,
    failedRuns: failures.length,
    completedToolCalls,
    durationMs: Number((performance.now() - started).toFixed(3)),
    failures,
  },
};

mkdirSync(dirname(outputPath), { recursive: true });
writeFileSync(outputPath, JSON.stringify(report, null, 2) + "\n");
console.log(
  "[runtime-reliability] " + report.result.successfulRuns + "/" + runs
    + " restart runs and " + completedToolCalls + " tool calls complete. Report: " + outputPath,
);
if (failures.length > 0) process.exit(1);
