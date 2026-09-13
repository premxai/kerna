[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$targetRoot = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $projectRoot 'kernel\target' }
$binary = @(
    (Join-Path $targetRoot 'debug\kerna.exe'),
    (Join-Path $targetRoot 'release\kerna.exe')
) | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
$reportDirectory = Join-Path $projectRoot 'reports\demo-readiness'
$reportPath = Join-Path $reportDirectory 'ci.json'

if (-not (Test-Path -LiteralPath $binary)) {
    throw "Expected built Kerna binary at $binary"
}

function Run-KernaCheck {
    param([string[]]$Arguments)
    & $binary @Arguments *> $null
    return $LASTEXITCODE -eq 0
}

$checks = [ordered]@{
    cli_help = Run-KernaCheck @('--help')
    demo_wizard_help = Run-KernaCheck @('init', '--demo', '--help')
    skills_surface = Run-KernaCheck @('skills')
    installer_present = Test-Path -LiteralPath (Join-Path $projectRoot 'scripts\install-hackathon.ps1')
    sponsor_bridge_present = Test-Path -LiteralPath (Join-Path $projectRoot 'runtime\sponsor-runtime.mjs')
}

$report = [ordered]@{
    kind = 'kerna-demo-ci-proof'
    generated_at_utc = [DateTime]::UtcNow.ToString('o')
    commit = (& git -C $projectRoot rev-parse --short HEAD).Trim()
    runner = $env:RUNNER_OS
    checks = $checks
    provider_keys_injected = $false
    note = 'CLI and packaging proof only; no provider keys, customer prompts, or sandbox workloads run in CI.'
    status = if (($checks.Values | Where-Object { -not $_ }).Count -eq 0) { 'pass' } else { 'fail' }
}

New-Item -ItemType Directory -Force -Path $reportDirectory | Out-Null
$report | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $reportPath -Encoding utf8
Write-Host "Demo CI proof: $($report.status)"
Write-Host "Report: $reportPath"
if ($report.status -ne 'pass') { exit 1 }
