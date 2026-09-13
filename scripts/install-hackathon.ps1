[CmdletBinding()]
param(
    [switch]$SkipDockerInstall,
    [switch]$SkipModelPull
)

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$installRoot = 'C:\KernaData'
$binRoot = Join-Path $installRoot 'bin'
$targetRoot = 'C:\Temp\kerna-target'

# Windows PowerShell sessions opened before Rust installation often do not have
# Cargo on PATH yet. Prefer the standard Rustup location before asking Winget
# to install an already-present toolchain.
$cargoCandidate = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
if (-not (Get-Command cargo -ErrorAction SilentlyContinue) -and (Test-Path -LiteralPath $cargoCandidate)) {
    $env:Path = "$(Split-Path -Parent $cargoCandidate);$env:Path"
}

function Require-WingetPackage {
    param([string]$Command, [string]$PackageId)
    if (Get-Command $Command -ErrorAction SilentlyContinue) { return }
    if (-not (Get-Command winget.exe -ErrorAction SilentlyContinue)) {
        throw "$Command is missing and winget is unavailable. Install $PackageId, then rerun."
    }
    winget.exe install --id $PackageId --exact --accept-source-agreements --accept-package-agreements --silent
    if ($LASTEXITCODE -ne 0) { throw "Failed to install $PackageId (exit $LASTEXITCODE)." }
    $machinePath = [Environment]::GetEnvironmentVariable('Path', 'Machine')
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $env:Path = "$machinePath;$userPath"
}

Require-WingetPackage -Command 'git' -PackageId 'Git.Git'
Require-WingetPackage -Command 'node' -PackageId 'OpenJS.NodeJS.LTS'
Require-WingetPackage -Command 'cargo' -PackageId 'Rustlang.Rustup'
if (-not $SkipDockerInstall) {
    Require-WingetPackage -Command 'docker' -PackageId 'Docker.DockerDesktop'
}

$cargoCommand = Get-Command cargo -ErrorAction SilentlyContinue
if (-not $cargoCommand) {
    if (-not (Test-Path -LiteralPath $cargoCandidate)) { throw 'Rust installed, but cargo is not available in this terminal.' }
    $cargoExecutable = $cargoCandidate
} else {
    $cargoExecutable = $cargoCommand.Source
}

& (Join-Path $PSScriptRoot 'bootstrap-demo.ps1') -SkipOllama:$SkipModelPull

New-Item -ItemType Directory -Force -Path $binRoot, $targetRoot | Out-Null
$env:CARGO_TARGET_DIR = $targetRoot
& $cargoExecutable build --manifest-path (Join-Path $projectRoot 'kernel\Cargo.toml') --release --locked
if ($LASTEXITCODE -ne 0) { throw "Kerna release build failed (exit $LASTEXITCODE)." }
try {
    Copy-Item -LiteralPath (Join-Path $targetRoot 'release\kerna.exe') -Destination (Join-Path $binRoot 'kerna.exe') -Force
} catch [System.IO.IOException] {
    throw 'Kerna is still running and Windows has locked C:\KernaData\bin\kerna.exe. Stop the Kerna dashboard or session, then rerun this installer.'
}

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (($userPath -split ';') -notcontains $binRoot) {
    [Environment]::SetEnvironmentVariable('Path', (($userPath.TrimEnd(';') + ';' + $binRoot).TrimStart(';')), 'User')
}
$env:Path = "$binRoot;$env:Path"

Write-Host ''
Write-Host 'Kerna is installed. The shortest workflow is:' -ForegroundColor Green
Write-Host '  kerna doctor'
Write-Host '  kerna'
& (Join-Path $binRoot 'kerna.exe') doctor --repo $projectRoot
