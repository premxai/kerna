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
    $inspection.Config.Labels.'dev.kerna.claude-code-version' -ne '2.1.270' -or
    $inspection.Config.Labels.'dev.kerna.claude-package' -ne '@anthropic-ai/claude-code@2.1.270' -or
    $inspection.Config.Labels.'dev.kerna.claude-package-integrity' -ne 'sha512-0zMkfIWQu7/SG56VP8r780HZWvrNShzK28AbAnhKRK0ns+ToGXPT0W8UqyZmZCUKAkJDd5//TrwSOhk1+hysiw==' -or
    $inspection.Config.Labels.'dev.kerna.rust-base-digest' -ne 'sha256:af306cfa71d987911a781c37b59d7d67d934f49684058f96cf72079c3626bfe0' -or
    $inspection.Config.Labels.'dev.kerna.node-base-digest' -ne 'sha256:8a34c4ab3ea2c5cd194f07e317b2a8f09461d3c8b05c4e34c8ccd56d56024c4d') {
    throw "Claude agent image verification failed"
}
Write-Output "$($inspection.Id) $($inspection.Config.Labels.'dev.kerna.contract') $($inspection.Config.Labels.'dev.kerna.claude-code-version') provenance=verified"
