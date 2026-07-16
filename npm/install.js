// Postinstall: fetch the prebuilt `kerna` binary for this platform from GitHub
// Releases and place it next to the launcher shim. Set KERNA_LOCAL_BIN to a
// local binary path to install offline (used for testing and air-gapped setups).
"use strict";

const fs = require("fs");
const path = require("path");
const https = require("https");
const crypto = require("crypto");
const { execFileSync } = require("child_process");

const REPO = "premxai/kerna";
const VERSION = process.env.KERNA_VERSION || require("./package.json").version;
const binDir = path.join(__dirname, "bin");
const isWindows = process.platform === "win32";
const outFile = path.join(binDir, isWindows ? "kerna.exe" : "kerna");
const pluginsZip = path.join(binDir, "kerna-plugins.zip");

function assetName() {
  const p = process.platform;
  const a = process.arch;
  if (p === "win32" && a === "x64") return "kerna-windows-x86_64.exe";
  if (p === "darwin" && a === "arm64") return "kerna-macos-arm64";
  if (p === "darwin" && a === "x64") return "kerna-macos-x86_64";
  if (p === "linux" && a === "x64") return "kerna-linux-x86_64";
  return null;
}

function fail(msg) {
  console.error("\x1b[31m[kerna] " + msg + "\x1b[0m");
  console.error(
    "[kerna] Install from source instead: cargo install --git https://github.com/" +
      REPO +
      " --bin kerna"
  );
  process.exit(1);
}

function download(url, dest, redirectsLeft) {
  return new Promise((resolve, reject) => {
    https
      .get(url, { headers: { "User-Agent": "kerna-npm-installer" } }, (res) => {
        if (
          res.statusCode >= 300 &&
          res.statusCode < 400 &&
          res.headers.location
        ) {
          if (redirectsLeft <= 0) return reject(new Error("too many redirects"));
          res.resume();
          return resolve(download(res.headers.location, dest, redirectsLeft - 1));
        }
        if (res.statusCode !== 200) {
          res.resume();
          return reject(new Error("HTTP " + res.statusCode + " for " + url));
        }
        const file = fs.createWriteStream(dest);
        res.pipe(file);
        file.on("finish", () => file.close(() => resolve()));
        file.on("error", reject);
      })
      .on("error", reject);
  });
}

function verifySha256(file, sidecar) {
  const expected = fs.readFileSync(sidecar, "utf8").trim().split(/\s+/)[0].toLowerCase();
  const actual = crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");
  if (!/^[a-f0-9]{64}$/.test(expected) || actual !== expected) {
    fail("SHA-256 verification failed for " + path.basename(file));
  }
}

async function downloadVerified(url, dest) {
  const sidecar = dest + ".sha256";
  try {
    await download(url, dest, 5);
    await download(url + ".sha256", sidecar, 5);
    verifySha256(dest, sidecar);
  } finally {
    fs.rmSync(sidecar, { force: true });
  }
}

function extractPlugins() {
  try {
    if (isWindows) {
      const quote = (value) => "'" + value.replace(/'/g, "''") + "'";
      execFileSync(
        "powershell.exe",
        [
          "-NoProfile",
          "-NonInteractive",
          "-Command",
          "Expand-Archive -LiteralPath " + quote(pluginsZip) + " -DestinationPath " + quote(binDir) + " -Force",
        ],
        { stdio: "inherit" }
      );
    } else {
      try {
        execFileSync(
          "python3",
          [
            "-c",
            "import sys,zipfile; zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])",
            pluginsZip,
            binDir,
          ],
          { stdio: "inherit" }
        );
      } catch {
        execFileSync("unzip", ["-oq", pluginsZip, "-d", binDir], { stdio: "inherit" });
      }
    }
  } catch (e) {
    fail("plugin bundle extraction failed: " + e.message);
  } finally {
    fs.rmSync(pluginsZip, { force: true });
  }

  if (!fs.existsSync(path.join(binDir, "plugins", "packs"))) {
    fail("plugin bundle did not contain curated packs");
  }
}

async function main() {
  fs.mkdirSync(binDir, { recursive: true });

  if (process.env.KERNA_LOCAL_BIN) {
    fs.copyFileSync(process.env.KERNA_LOCAL_BIN, outFile);
    if (!isWindows) fs.chmodSync(outFile, 0o755);
    console.log("[kerna] Installed from local binary: " + outFile);
    return;
  }

  const asset = assetName();
  if (!asset) {
    fail(
      "no prebuilt binary for " + process.platform + "/" + process.arch + "."
    );
  }

  const url =
    "https://github.com/" +
    REPO +
    "/releases/download/v" +
    VERSION +
    "/" +
    asset;

  console.log("[kerna] Downloading " + asset + " (v" + VERSION + ")...");
  try {
    await downloadVerified(url, outFile);
    if (!isWindows) fs.chmodSync(outFile, 0o755);
    console.log("[kerna] Installed: " + outFile);

    const pluginsUrl =
      "https://github.com/" +
      REPO +
      "/releases/download/v" +
      VERSION +
      "/kerna-plugins.zip";
    console.log("[kerna] Downloading curated plugins (v" + VERSION + ")...");
    await downloadVerified(pluginsUrl, pluginsZip);
    extractPlugins();
    console.log("[kerna] Installed curated plugins: " + path.join(binDir, "plugins"));
  } catch (e) {
    fail("download failed: " + e.message + " (is release v" + VERSION + " published?)");
  }
}

main().catch((e) => fail(e.message));
