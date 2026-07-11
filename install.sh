#!/bin/sh
# Kerna installer for macOS and Linux.
#
#   curl -fsSL https://raw.githubusercontent.com/premxai/kerna/main/install.sh | sh
#
# Env overrides:
#   KERNA_VERSION   tag to install (default: latest)
#   KERNA_BIN_DIR   install directory (default: $HOME/.local/bin)
#   KERNA_LOCAL_BIN path to a local kerna binary to install instead of downloading
set -eu

REPO="premxai/kerna"
BIN_DIR="${KERNA_BIN_DIR:-$HOME/.local/bin}"
VERSION="${KERNA_VERSION:-latest}"

say()  { printf '\033[36m%s\033[0m\n' "$*"; }
err()  { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }

# --- detect platform --------------------------------------------------------
os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
  Linux)  os="linux" ;;
  Darwin) os="macos" ;;
  *) err "unsupported OS '$os'. Install from source: cargo install --git https://github.com/$REPO --bin kerna" ;;
esac
case "$arch" in
  x86_64|amd64)  arch="x86_64" ;;
  arm64|aarch64) arch="arm64" ;;
  *) err "unsupported architecture '$arch'. Install from source: cargo install --git https://github.com/$REPO --bin kerna" ;;
esac

# Only macOS ships an arm64 prebuilt today; Linux arm64 builds from source.
if [ "$os" = "linux" ] && [ "$arch" = "arm64" ]; then
  err "no prebuilt Linux arm64 binary yet. Install from source: cargo install --git https://github.com/$REPO --bin kerna"
fi

asset="kerna-${os}-${arch}"
target="$BIN_DIR/kerna"

mkdir -p "$BIN_DIR"

# --- fetch ------------------------------------------------------------------
if [ -n "${KERNA_LOCAL_BIN:-}" ]; then
  say "Installing kerna from local binary $KERNA_LOCAL_BIN"
  cp "$KERNA_LOCAL_BIN" "$target"
else
  if [ "$VERSION" = "latest" ]; then
    url="https://github.com/$REPO/releases/latest/download/$asset"
  else
    url="https://github.com/$REPO/releases/download/$VERSION/$asset"
  fi
  say "Downloading kerna ($os/$arch) from $url"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url" -o "$target" || err "download failed (is there a published release yet?)"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "$target" "$url" || err "download failed (is there a published release yet?)"
  else
    err "need curl or wget to download"
  fi
fi

chmod +x "$target"

# --- verify + PATH hint -----------------------------------------------------
say "Installed: $target"
"$target" --version || err "the installed binary did not run"

case ":$PATH:" in
  *":$BIN_DIR:"*) : ;;
  *)
    printf '\n\033[33mNote:\033[0m %s is not on your PATH. Add it:\n' "$BIN_DIR"
    printf '  echo '\''export PATH="%s:$PATH"'\'' >> ~/.bashrc   # or ~/.zshrc\n' "$BIN_DIR"
    ;;
esac

printf '\nGet started:  \033[1mkerna init\033[0m\n'
