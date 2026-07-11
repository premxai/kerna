#!/usr/bin/env node
// Launcher: exec the platform `kerna` binary that postinstall placed next to
// this file, forwarding all arguments, stdio, and the exit code.
"use strict";

const path = require("path");
const fs = require("fs");
const { spawnSync } = require("child_process");

const isWindows = process.platform === "win32";
const bin = path.join(__dirname, isWindows ? "kerna.exe" : "kerna");

if (!fs.existsSync(bin)) {
  console.error(
    "\x1b[31m[kerna] binary not found. Reinstall with `npm install -g @premxai/kerna`,\x1b[0m"
  );
  console.error(
    "[kerna] or build from source: cargo install --git https://github.com/premxai/kerna --bin kerna"
  );
  process.exit(1);
}

const result = spawnSync(bin, process.argv.slice(2), { stdio: "inherit" });
if (result.error) {
  console.error("[kerna] failed to launch: " + result.error.message);
  process.exit(1);
}
process.exit(result.status === null ? 1 : result.status);
