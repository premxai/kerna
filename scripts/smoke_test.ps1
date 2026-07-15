$ErrorActionPreference = "Stop"

Write-Host "======================================"
Write-Host "Kerna Trust Layer Smoke Test (Windows)"
Write-Host "======================================"

if (-not $env:KERNA_BIN) {
    Write-Host "Compiling Kerna..."
    cargo build --bin kerna
    $KernaBin = ".\target\debug\kerna.exe"
} else {
    $KernaBin = $env:KERNA_BIN
}
$KernaBin = (Resolve-Path -LiteralPath $KernaBin).Path
$McpCommand = $KernaBin.Replace("\", "\\")

if (Test-Path "kerna.toml") { Remove-Item -Force "kerna.toml" }

Set-Content -Path "kerna.toml" -Value @"
db_path = `"kerna.db`"
sandbox_dir = `"./sandbox`"
memory_backend = `"sqlite`"
llm_model = `"gpt-4o`"
llm_provider = `"mock`"
llm_api_key = `"fake`"

[[mcp_servers]]
name = `"mockmcp`"
command = `"$McpCommand`"
args = [`"mockmcp`"]
enabled = true
capabilities = [`"echo`"]
"@

Add-Content -Path "kerna.toml" -Value @"

[[permissions]]
tool = "echo"
action = "auto_approve"

[[permissions]]
tool = "*"
action = "deny"
"@


Write-Host "[1/4] Running Kerna Doctor..."
& $KernaBin doctor

Write-Host "[2/4] Verifying MockMCP..."
# Run mockmcp briefly to ensure it compiles and starts
if (echo '{"jsonrpc":"2.0","method":"tools/list","id":1}' | & $KernaBin mockmcp | Out-String -Stream | Select-String "echo") {
    Write-Host "[+] MockMCP tools/list successful."
} else {
    Write-Host "[-] MockMCP failed."
    exit 1
}

Write-Host "[3/4] Running an agent goal..."
$env:KERNA_MOCK_LLM = "1"
Write-Host "Running goal to test tool execution..."
& $KernaBin run "Please call echo" | Out-File run_output.txt
Get-Content run_output.txt

# Extract Task ID from output
$TaskId = (Select-String -Path run_output.txt -Pattern "Task completed:").Line -replace ".*Task completed: (.*)", "`$1"

if (-not $TaskId) {
    Write-Host "[-] Failed to extract Task ID. Was the goal successful?"
    exit 1
}
Write-Host "[+] Goal completed with Task ID: $TaskId"

Write-Host "[4/4] Verifying Trace..."
& $KernaBin trace $TaskId | Out-File trace_output.txt
Get-Content trace_output.txt

# Verify that the explicitly granted tool actually completed.
if (Select-String -Path trace_output.txt -Pattern "tool.call.completed" -Quiet) {
    Write-Host "[+] Allowed tool execution and trace verified successfully."
} else {
    Write-Host "[-] Trace missing a completed allowed tool call!"
    exit 1
}

Write-Host "======================================"
Write-Host "All smoke tests passed successfully!"
Write-Host "======================================"
Remove-Item run_output.txt, trace_output.txt
