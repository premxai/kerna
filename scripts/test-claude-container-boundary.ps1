$ErrorActionPreference = "Stop"
$docker = if (Test-Path "C:\Program Files\Docker\Docker\resources\bin\docker.exe") {
    "C:\Program Files\Docker\Docker\resources\bin\docker.exe"
} else { "docker" }
$suffix = [Guid]::NewGuid().ToString("N").Substring(0, 8)
$agentNetwork = "kerna-proof-agent-$suffix"
$egressNetwork = "kerna-proof-egress-$suffix"
$broker = "kerna-proof-broker-$suffix"
$workspace = Join-Path ([System.IO.Path]::GetTempPath()) "kerna-proof-$suffix"
$image = "kerna-claude-agent:0.2.9-claude-2.1.270"

New-Item -ItemType Directory -Path $workspace | Out-Null
Set-Content -LiteralPath (Join-Path $workspace "workspace-canary.txt") -Value "workspace-only"
try {
    & $docker network create --internal --label dev.kerna.managed=true $agentNetwork | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "could not create internal agent network" }
    & $docker network create --label dev.kerna.managed=true $egressNetwork | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "could not create broker egress network" }
    & $docker run --detach --rm --name $broker --network $agentNetwork $image node -e "require('http').createServer((q,r)=>r.end('broker-ok')).listen(8766,'0.0.0.0')" | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "could not start proof broker" }
    & $docker network connect $egressNetwork $broker
    if ($LASTEXITCODE -ne 0) { throw "could not attach proof broker egress" }

    $probe = @'
const fs = require('fs');
const fail = m => { console.error(m); process.exit(1); };
(async () => {
  if (!fs.existsSync('/workspace/workspace-canary.txt')) fail('workspace mount missing');
  if (fs.existsSync('/var/run/docker.sock')) fail('docker socket exposed');
  if (fs.existsSync('/host-home-canary')) fail('host home exposed');
  if (process.env.ANTHROPIC_API_KEY || process.env.TENKI_API_KEY) fail('provider credential exposed');
  const mounts = fs.readFileSync('/proc/mounts', 'utf8');
  if (mounts.includes('/host_mnt') || mounts.includes('/run/desktop/mnt/host')) fail('broad host mount exposed');
  fs.writeFileSync('/workspace/agent-write.txt', 'contained');
  const broker = await fetch('http://kerna-proof-broker-SUFFIX:8766').then(r => r.text());
  if (broker !== 'broker-ok') fail('broker-only route unavailable');
  let escaped = false;
  try { await fetch('https://example.com', {signal: AbortSignal.timeout(2000)}); escaped = true; } catch (_) {}
  if (escaped) fail('agent reached public network');
  console.log(JSON.stringify({workspace:true, host_home:false, docker_socket:false, provider_key:false, broker:true, public_network:false, uid:process.getuid()}));
})().catch(error => fail(error.message));
'@.Replace('SUFFIX', $suffix)

    & $docker run --rm --network $agentNetwork --read-only --cap-drop ALL --security-opt no-new-privileges:true --pids-limit 64 --memory 512m --tmpfs /tmp:rw,noexec,nosuid,nodev,size=64m,mode=1777 --mount "type=bind,src=$workspace,dst=/workspace" --workdir /workspace --env HOME=/tmp/kerna $image node -e $probe
    if ($LASTEXITCODE -ne 0) { throw "contained agent boundary proof failed" }
    if (!(Test-Path (Join-Path $workspace "agent-write.txt"))) { throw "workspace write was not observed" }
} finally {
    & $docker rm --force $broker 2>$null | Out-Null
    & $docker network rm $agentNetwork $egressNetwork 2>$null | Out-Null
    Remove-Item -LiteralPath $workspace -Recurse -Force -ErrorAction SilentlyContinue
}
