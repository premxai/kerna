#!/bin/bash
set -e

echo "======================================"
echo "Kerna Trust Layer Smoke Test (Linux/macOS)"
echo "======================================"

if [ -z "${KERNA_BIN:-}" ]; then
  echo "Compiling Kerna..."
  cargo build --manifest-path kernel/Cargo.toml --bin kerna
  KERNA_BIN="$(pwd)/target/debug/kerna"
else
  KERNA_BIN="$(cd "$(dirname "$KERNA_BIN")" && pwd)/$(basename "$KERNA_BIN")"
fi

if [ -f "kerna.toml" ]; then
    rm -f kerna.toml
fi

cat << EOF > kerna.toml
db_path = "kerna.db"
sandbox_dir = "./sandbox"
memory_backend = "sqlite"
llm_model = "gpt-4o"
llm_provider = "mock"
llm_api_key = "fake"

[[mcp_servers]]
name = "mockmcp"
command = "$KERNA_BIN"
args = ["mockmcp"]
enabled = true
capabilities = ["echo"]
EOF

cat >> kerna.toml <<'EOF'

[[permissions]]
tool = "echo"
action = "auto_approve"

[[permissions]]
tool = "*"
action = "deny"
EOF

echo "[1/4] Running Kerna Doctor..."
"$KERNA_BIN" doctor

echo "[2/4] Verifying MockMCP..."
# Run mockmcp briefly to ensure it compiles and starts
echo -e '{"jsonrpc":"2.0","method":"tools/list","id":1}' | "$KERNA_BIN" mockmcp run | grep -q "echo"
echo "[+] MockMCP tools/list successful."

echo "[3/4] Running an agent goal..."
# We will use converse=false implicitly
# kerna run ...
echo "Running goal to test tool execution..."
"$KERNA_BIN" run "Please call echo" > run_output.txt
cat run_output.txt

# Extract Task ID from output
TASK_ID=$(grep "Task completed:" run_output.txt | awk '{print $NF}')

if [ -z "$TASK_ID" ]; then
    echo "[-] Failed to extract Task ID. Was the goal successful?"
    exit 1
fi
echo "[+] Goal completed with Task ID: $TASK_ID"

echo "[4/4] Verifying Trace..."
"$KERNA_BIN" trace "$TASK_ID" > trace_output.txt
cat trace_output.txt

# Verify that the explicitly granted tool actually completed.
if grep -q "tool.call.completed" trace_output.txt; then
    echo "[+] Allowed tool execution and trace verified successfully."
else
    echo "[-] Trace missing a completed allowed tool call!"
    exit 1
fi

echo "======================================"
echo "All smoke tests passed successfully!"
echo "======================================"
rm run_output.txt trace_output.txt
