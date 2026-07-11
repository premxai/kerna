#!/bin/sh
# Register the Kerna starter plugin pack (files, web, git) into the kerna.toml
# in the current directory. Run from your project dir after `kerna init`.
set -eu

# Absolute path to this repo's plugins/ dir (works regardless of where you run from).
SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
PLUGINS="$SCRIPT_DIR/../plugins"

KERNA="${KERNA:-kerna}"

for p in files web git; do
  "$KERNA" mcp add "$p" python "$PLUGINS/${p}_mcp/mcp_server.py"
done

echo
echo "Added: files, web, git. Next:"
echo "  $KERNA mcp list            # confirm they loaded"
echo "  $KERNA mcp risk files      # see the risk card"
echo "  then grant tools in kerna.toml (fail-closed by default)"
