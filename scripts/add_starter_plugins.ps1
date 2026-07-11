# Register the Kerna starter plugin pack (files, web, git) into the kerna.toml
# in the current directory. Run from your project dir after `kerna init`.
$ErrorActionPreference = 'Stop'

$plugins = Join-Path (Split-Path -Parent $PSScriptRoot) 'plugins'
$kerna = if ($env:KERNA) { $env:KERNA } else { 'kerna' }

foreach ($p in @('files', 'web', 'git')) {
    & $kerna mcp add $p python (Join-Path $plugins "$($p)_mcp\mcp_server.py")
}

Write-Host ""
Write-Host "Added: files, web, git. Next:"
Write-Host "  $kerna mcp list            # confirm they loaded"
Write-Host "  $kerna mcp risk files      # see the risk card"
Write-Host "  then grant tools in kerna.toml (fail-closed by default)"
