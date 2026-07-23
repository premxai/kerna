import { existsSync, mkdirSync, rmSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(scriptDir, "..", "..");
const outIndex = process.argv.indexOf("--out");
if (outIndex !== -1 && !process.argv[outIndex + 1]) {
  throw new Error("--out requires a directory");
}
const outputDir = resolve(
  repoRoot,
  outIndex === -1 ? "reports/mcp-conformance/latest" : process.argv[outIndex + 1],
);
const binary = resolve(
  repoRoot,
  "target",
  "debug",
  process.platform === "win32" ? "kerna.exe" : "kerna",
);
const conformanceVersion = "0.1.16";
const scenarios = ["initialize", "tools_call"];
const npxCommand = process.platform === "win32" ? process.execPath : "npx";
const npxPrefix = process.platform === "win32"
  ? [resolve(dirname(process.execPath), "node_modules", "npm", "bin", "npx-cli.js")]
  : [];

function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: repoRoot,
    encoding: "utf8",
    stdio: "inherit",
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(command + " " + args.join(" ") + " failed with exit code " + result.status);
  }
}

run("cargo", ["build", "--manifest-path", "kernel/Cargo.toml"]);
if (!existsSync(binary)) throw new Error("Expected Kerna binary at " + binary);

rmSync(outputDir, { recursive: true, force: true });
mkdirSync(outputDir, { recursive: true });
const command = '"' + binary + '" mcp conformance-client';

for (const scenario of scenarios) {
  run(npxCommand, [
    ...npxPrefix,
    "--yes",
    "@modelcontextprotocol/conformance@" + conformanceVersion,
    "client",
    "--command",
    command,
    "--scenario",
    scenario,
    "--spec-version",
    "2025-06-18",
    "--timeout",
    "60000",
    "--output-dir",
    resolve(outputDir, scenario),
  ]);
}

console.log(
  "[mcp-conformance] " + scenarios.length + "/" + scenarios.length
    + " official core client scenarios passed. Report: " + outputDir,
);
