# Kerna installer for Windows.
#
#   irm https://raw.githubusercontent.com/premxai/kerna/main/install.ps1 | iex
#
# Env overrides:
#   KERNA_VERSION    tag to install (default: latest)
#   KERNA_BIN_DIR    install directory (default: %USERPROFILE%\.local\bin)
#   KERNA_LOCAL_BIN  path to a local kerna.exe to install instead of downloading
$ErrorActionPreference = 'Stop'

$repo   = 'premxai/kerna'
$binDir = if ($env:KERNA_BIN_DIR) { $env:KERNA_BIN_DIR } else { Join-Path $env:USERPROFILE '.local\bin' }
$version = if ($env:KERNA_VERSION) { $env:KERNA_VERSION } else { 'latest' }
$asset  = 'kerna-windows-x86_64.exe'
$target = Join-Path $binDir 'kerna.exe'

New-Item -ItemType Directory -Force -Path $binDir | Out-Null

if ($env:KERNA_LOCAL_BIN) {
    Write-Host "Installing kerna from local binary $($env:KERNA_LOCAL_BIN)" -ForegroundColor Cyan
    Copy-Item -Path $env:KERNA_LOCAL_BIN -Destination $target -Force
} else {
    $url = if ($version -eq 'latest') {
        "https://github.com/$repo/releases/latest/download/$asset"
    } else {
        "https://github.com/$repo/releases/download/$version/$asset"
    }
    Write-Host "Downloading kerna (windows/x86_64) from $url" -ForegroundColor Cyan
    try {
        Invoke-WebRequest -Uri $url -OutFile $target -UseBasicParsing
    } catch {
        Write-Error "Download failed (is there a published release yet?). $_"
    }
}

Write-Host "Installed: $target" -ForegroundColor Cyan
& $target --version

# Curated packs are external MCP processes, shipped as a release bundle rather
# than compiled into the trust-layer binary. Keep them beside the executable so
# the CLI discovers them without requiring KERNA_PLUGINS_DIR.
$pluginsAsset = 'kerna-plugins.zip'
$pluginsZip = Join-Path $binDir $pluginsAsset
$pluginsUrl = if ($version -eq 'latest') {
    "https://github.com/$repo/releases/latest/download/$pluginsAsset"
} else {
    "https://github.com/$repo/releases/download/$version/$pluginsAsset"
}
Write-Host "Downloading curated Kerna plugins from $pluginsUrl" -ForegroundColor Cyan
try {
    Invoke-WebRequest -Uri $pluginsUrl -OutFile $pluginsZip -UseBasicParsing
    Expand-Archive -LiteralPath $pluginsZip -DestinationPath $binDir -Force
} catch {
    Write-Error "Plugin bundle download or extraction failed. $_"
} finally {
    Remove-Item -LiteralPath $pluginsZip -Force -ErrorAction SilentlyContinue
}
if (-not (Test-Path -LiteralPath (Join-Path $binDir 'plugins\packs'))) {
    Write-Error "Curated plugins were not installed."
}

# Add to the user PATH if it isn't already there.
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if ($userPath -notlike "*$binDir*") {
    [Environment]::SetEnvironmentVariable('Path', "$userPath;$binDir", 'User')
    Write-Host "Added $binDir to your user PATH. Open a new terminal to use 'kerna'." -ForegroundColor Yellow
}

Write-Host "`nGet started:  kerna init" -ForegroundColor Green
