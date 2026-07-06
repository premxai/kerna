#!/bin/bash
set -e

echo "======================================"
echo "Kerna Trust Layer Smoke Test (Linux/macOS)"
echo "======================================"

KERNA_BIN=${KERNA_BIN:-"cargo run --bin kerna --"}

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
command = "./target/debug/kerna"
args = ["mockmcp"]
enabled = true
capabilities = ["echo"]
EOF

echo "[1/4] Running Kerna Doctor..."
$KERNA_BIN doctor

echo "[2/4] Verifying MockMCP..."
# Run mockmcp briefly to ensure it compiles and starts
echo -e '{"jsonrpc":"2.0","method":"tools/list","id":1}' | $KERNA_BIN mockmcp run | grep -q "echo"
echo "[+] MockMCP tools/list successful."

echo "[3/4] Running an agent goal..."
# We will use converse=false implicitly
# kerna run ...
echo "Running goal to test tool execution..."
$KERNA_BIN run "Please call echo" > run_output.txt || true
cat run_output.txt

# Extract Task ID from output
TASK_ID=$(grep "Task completed:" run_output.txt | awk '{print $NF}')

if [ -z "$TASK_ID" ]; then
    echo "[-] Failed to extract Task ID. Was the goal successful?"
    exit 1
fi
echo "[+] Goal completed with Task ID: $TASK_ID"

echo "[4/4] Verifying Trace..."
$KERNA_BIN trace $TASK_ID > trace_output.txt
cat trace_output.txt

# Verify that pipeline events are in the trace
if grep -q "tool.call.requested" trace_output.txt; then
    echo "[+] Pipeline trace verified successfully."
else
    echo "[-] Trace missing pipeline events!"
    exit 1
fi

echo "======================================"
echo "All smoke tests passed successfully!"
echo "======================================"
rm run_output.txt trace_output.txt
