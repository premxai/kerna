param(
    [string]$TargetDir = "C:\Temp\kerna-security-boundary"
)

$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot
$kernel = Join-Path $repo "kernel"
$env:CARGO_TARGET_DIR = $TargetDir

function Invoke-Probe([string]$Filter) {
    Write-Host "[probe] $Filter"
    & cargo test --locked $Filter -- --exact --nocapture
    if ($LASTEXITCODE -ne 0) {
        throw "Security boundary probe failed: $Filter"
    }
}

Push-Location $kernel
try {
    Invoke-Probe "guard_protocol::tests::malformed_arguments_fail_closed"
    Invoke-Probe "folders::tests::safe_join_rejects_symlink_escape"
    Invoke-Probe "trust_layer_validation::test_declared_secret_reaches_plugin_undeclared_does_not"
    Invoke-Probe "trust_layer_validation::test_queued_approval_is_recorded_before_tool_execution"
    Invoke-Probe "gateway::tests::contained_filesystem_fixture_reads_and_approval_gates_real_writes"
    Invoke-Probe "server::tests::demo_sandbox_denials_have_auditable_receipts_on_both_routes"
    Invoke-Probe "sponsor_runtime::tests::kerna_denies_escape_intent_before_backend_contact"
    Invoke-Probe "sandbox::tests::docker_mode_without_docker_explains_itself"
    & cargo test --locked --test security_action_inventory
    if ($LASTEXITCODE -ne 0) {
        throw "Security action inventory gate failed"
    }
} finally {
    Pop-Location
}

Write-Host "Security boundary probes passed. All payloads were inert test fixtures."
