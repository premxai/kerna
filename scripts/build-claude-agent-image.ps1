$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot
$image = "kerna-claude-agent:0.2.9-claude-2.1.270"
$docker = if (Test-Path "C:\Program Files\Docker\Docker\resources\bin\docker.exe") {
    "C:\Program Files\Docker\Docker\resources\bin\docker.exe"
} else {
    "docker"
}

& $docker build --pull=false --file (Join-Path $repo "docker\claude-agent\Dockerfile") --tag $image $repo
if ($LASTEXITCODE -ne 0) { throw "Claude agent image build failed" }
$inspection = (& $docker image inspect $image | ConvertFrom-Json)[0]
if ($LASTEXITCODE -ne 0 -or
    $inspection.Config.Labels.'dev.kerna.contract' -ne 'claude-agent-v1' -or
    $inspection.Config.Labels.'dev.kerna.claude-code-version' -ne '2.1.270') {
    throw "Claude agent image verification failed"
}
Write-Output "$($inspection.Id) $($inspection.Config.Labels.'dev.kerna.contract') $($inspection.Config.Labels.'dev.kerna.claude-code-version')"
