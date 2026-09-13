[CmdletBinding()]
param(
    [switch]$SkipOllama,
    [switch]$SkipClaudePrewarm
)

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$runtimeSource = Join-Path $projectRoot 'runtime'
$demoRoot = 'C:\KernaData\kerna-demo'
$runtimeTarget = Join-Path $demoRoot 'runtime'
$npmCache = Join-Path $demoRoot 'npm-cache'
$ollamaModels = 'C:\KernaData\ollama-models'

New-Item -ItemType Directory -Force -Path $runtimeTarget, $npmCache, $ollamaModels, 'C:\Temp\kerna-target', 'C:\Temp\kerna-sessions' | Out-Null
Copy-Item -LiteralPath (Join-Path $runtimeSource 'package.json') -Destination $runtimeTarget -Force
Copy-Item -LiteralPath (Join-Path $runtimeSource 'package-lock.json') -Destination $runtimeTarget -Force
Copy-Item -LiteralPath (Join-Path $runtimeSource 'sponsor-runtime.mjs') -Destination $runtimeTarget -Force
npm.cmd ci --prefix $runtimeTarget --cache $npmCache --ignore-scripts --no-audit --no-fund
if ($LASTEXITCODE -ne 0) { throw "npm ci failed with exit code $LASTEXITCODE" }

if (-not $SkipOllama) {
    $ollamaCommand = Get-Command ollama -ErrorAction SilentlyContinue
    $ollamaPath = if ($ollamaCommand) { $ollamaCommand.Source } else { $null }
    if (-not $ollamaPath) {
        $candidate = Join-Path $env:LOCALAPPDATA 'Programs\Ollama\ollama.exe'
        if (Test-Path -LiteralPath $candidate) {
            $ollamaPath = $candidate
        } else {
            winget.exe install --id Ollama.Ollama --exact --accept-source-agreements --accept-package-agreements --silent
            if ($LASTEXITCODE -ne 0) { throw "Ollama installation failed with exit code $LASTEXITCODE" }
            $ollamaPath = $candidate
        }
    }
    [Environment]::SetEnvironmentVariable('OLLAMA_MODELS', $ollamaModels, 'User')
    $env:OLLAMA_MODELS = $ollamaModels
    Get-Process ollama -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Process -FilePath $ollamaPath -ArgumentList 'serve' -WindowStyle Hidden -Environment @{ OLLAMA_MODELS = $ollamaModels }
    Start-Sleep -Seconds 3
    & $ollamaPath pull qwen2.5-coder:7b
    if ($LASTEXITCODE -ne 0) { throw "Ollama model pull failed with exit code $LASTEXITCODE" }
}

if (-not $SkipClaudePrewarm) {
    npm.cmd --cache $npmCache exec --yes --package '@anthropic-ai/claude-code@2.1.270' -- claude --version
    if ($LASTEXITCODE -ne 0) { throw "Claude Code prewarm failed with exit code $LASTEXITCODE" }
}

Write-Host 'Kerna demo dependencies are installed on C:.'
Write-Host 'Run: kerna guard doctor --demo --repo .'
