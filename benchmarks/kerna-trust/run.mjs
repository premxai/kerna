#!/usr/bin/env node

import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const repositoryRoot = resolve(scriptDirectory, "../..");
const manifestPath = join(scriptDirectory, "scenarios.json");
const defaultOutput = join(repositoryRoot, "reports", "kerna-trust", "latest.json");
const argumentsList = process.argv.slice(2);

function option(name) {
  const index = argumentsList.indexOf(name);
  return index >= 0 ? argumentsList[index + 1] : undefined;
}

if (argumentsList.includes("--help")) {
  console.log("Usage: node benchmarks/kerna-trust/run.mjs [--category <name>] [--out <path>]");
  process.exit(0);
}

const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
const category = option("--category");
const outputPath = resolve(repositoryRoot, option("--out") ?? defaultOutput);
const selectedScenarios = manifest.scenarios.filter((scenario) => !category || scenario.category === category);

if (selectedScenarios.length === 0) {
  console.error(`No Kerna Trust Bench scenarios matched category: ${category}`);
  process.exit(2);
}

// Integration tests spawn the compiled `kerna mockmcp` executable as their
// external stdio process. Build it first so a standalone benchmark run cannot
// accidentally exercise a binary left over from an older source revision.
process.stdout.write("[kerna-trust] Building Kerna child-process fixture ... ");
const build = spawnSync("cargo", ["build", "--bin", "kerna"], {
  cwd: join(repositoryRoot, "kernel"),
  encoding: "utf8",
  timeout: 120_000
});
if (build.status !== 0 || build.error) {
  const output = `${build.stdout ?? ""}${build.stderr ?? ""}`.trim();
  console.log("FAILED");
  console.error(output || build.error?.message || "cargo build failed");
  process.exit(1);
}
console.log("ready");

const startedAt = new Date();
const results = [];

for (const scenario of selectedScenarios) {
  process.stdout.write(`\n[kerna-trust] ${scenario.id} ... `);
  const started = performance.now();
  const processResult = spawnSync(
    "cargo",
    ["test", "--bin", "kerna", scenario.test, "--", "--exact"],
    {
      cwd: join(repositoryRoot, "kernel"),
      encoding: "utf8",
      timeout: 120_000
    }
  );
  const durationMs = Math.round(performance.now() - started);
  const passed = processResult.status === 0 && !processResult.error;
  const output = `${processResult.stdout ?? ""}${processResult.stderr ?? ""}`.trim();

  results.push({
    id: scenario.id,
    category: scenario.category,
    test: scenario.test,
    claim: scenario.claim,
    status: passed ? "passed" : "failed",
    durationMs,
    failureExcerpt: passed ? undefined : output.slice(-2_000)
  });

  console.log(passed ? `passed (${durationMs}ms)` : `FAILED (${durationMs}ms)`);
}

const passed = results.filter((result) => result.status === "passed").length;
const report = {
  benchmark: manifest.name,
  version: manifest.version,
  deterministic: true,
  startedAt: startedAt.toISOString(),
  completedAt: new Date().toISOString(),
  summary: {
    total: results.length,
    passed,
    failed: results.length - passed,
    passRate: Number((passed / results.length).toFixed(4))
  },
  scenarios: results
};

mkdirSync(dirname(outputPath), { recursive: true });
writeFileSync(outputPath, `${JSON.stringify(report, null, 2)}\n`);
console.log(`\n[kerna-trust] ${passed}/${results.length} passed. Report: ${outputPath}`);

if (passed !== results.length) process.exit(1);
