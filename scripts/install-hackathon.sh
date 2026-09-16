#!/usr/bin/env sh
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
data_root="${XDG_DATA_HOME:-$HOME/.local/share}/kerna-demo"
runtime_root="$data_root/runtime"
bin_root="${KERNA_BIN_DIR:-$HOME/.local/bin}"
target_root="${KERNA_CARGO_TARGET:-${TMPDIR:-/tmp}/kerna-target}"

need() { command -v "$1" >/dev/null 2>&1 || { printf 'Missing requirement: %s\n' "$1" >&2; exit 1; }; }
need git
need cargo
need node
need npm
need docker
need ollama

mkdir -p "$runtime_root" "$data_root/npm-cache" "$data_root/wasmer-cache" "$bin_root" "$target_root"
cp "$project_root/runtime/package.json" "$runtime_root/package.json"
cp "$project_root/runtime/package-lock.json" "$runtime_root/package-lock.json"
cp "$project_root/runtime/sponsor-runtime.mjs" "$runtime_root/sponsor-runtime.mjs"
npm ci --prefix "$runtime_root" --cache "$data_root/npm-cache" --ignore-scripts --no-audit --no-fund
npm --cache "$data_root/npm-cache" exec --yes --package '@anthropic-ai/claude-code@2.1.270' -- claude --version
ollama pull qwen3.5:9b

CARGO_TARGET_DIR="$target_root" cargo build --manifest-path "$project_root/kernel/Cargo.toml" --release --locked
cp "$target_root/release/kerna" "$bin_root/kerna"
chmod +x "$bin_root/kerna"

printf '\nKerna is installed at %s.\n' "$bin_root/kerna"
printf 'Run: kerna doctor\nThen: kerna\n'
"$bin_root/kerna" doctor --repo "$project_root"
