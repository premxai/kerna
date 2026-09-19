#!/usr/bin/env bash
set -euo pipefail

suffix="$(printf '%08x' "$RANDOM$RANDOM")"
agent_network="kerna-proof-agent-$suffix"
egress_network="kerna-proof-egress-$suffix"
broker="kerna-proof-broker-$suffix"
workspace="$(mktemp -d "${TMPDIR:-/tmp}/kerna-proof.XXXXXXXX")"
image="kerna-claude-agent:0.2.9-claude-2.1.270"
cleanup() {
  docker rm --force "$broker" >/dev/null 2>&1 || true
  docker network rm "$agent_network" "$egress_network" >/dev/null 2>&1 || true
  rm -rf -- "$workspace"
}
trap cleanup EXIT

printf 'workspace-only\n' > "$workspace/workspace-canary.txt"
docker network create --internal --label dev.kerna.managed=true "$agent_network" >/dev/null
docker network create --label dev.kerna.managed=true "$egress_network" >/dev/null
docker run --detach --rm --name "$broker" --network "$agent_network" "$image" \
  node -e "require('http').createServer((q,r)=>r.end('broker-ok')).listen(8766,'0.0.0.0')" >/dev/null
docker network connect "$egress_network" "$broker"

probe="$(cat <<'JS'
const fs = require('fs');
const fail = m => { console.error(m); process.exit(1); };
(async () => {
  if (!fs.existsSync('/workspace/workspace-canary.txt')) fail('workspace mount missing');
  if (fs.existsSync('/var/run/docker.sock')) fail('docker socket exposed');
  if (fs.existsSync('/host-home-canary')) fail('host home exposed');
  if (fs.existsSync('/kerna-state') || fs.existsSync('/workspace/.kerna-demo/kerna-demo.db')) fail('trusted evidence store exposed');
  const forbiddenCredentials = ['ANTHROPIC_API_KEY', 'OPENAI_API_KEY', 'TENKI_API_KEY', 'KERNA_BROWSER_CONTROL_TOKEN', 'KERNA_UNRELATED_PLUGIN_SECRET'];
  if (forbiddenCredentials.some(name => process.env[name])) fail('provider, browser, or unrelated plugin credential exposed');
  const mounts = fs.readFileSync('/proc/mounts', 'utf8');
  if (mounts.includes('/host_mnt') || mounts.includes('/run/desktop/mnt/host')) fail('broad host mount exposed');
  fs.writeFileSync('/workspace/agent-write.txt', 'contained');
  const broker = await fetch('http://BROKER_NAME:8766').then(r => r.text());
  if (broker !== 'broker-ok') fail('broker-only route unavailable');
  let escaped = false;
  try { await fetch('https://example.com', {signal: AbortSignal.timeout(2000)}); escaped = true; } catch (_) {}
  if (escaped) fail('agent reached public network');
  console.log(JSON.stringify({workspace:true,evidence_store:false,host_home:false,docker_socket:false,provider_key:false,browser_key:false,unrelated_plugin_key:false,broker:true,public_network:false,uid:process.getuid()}));
})().catch(error => fail(error.message));
JS
)"
probe="${probe//BROKER_NAME/$broker}"

docker run --rm --network "$agent_network" --read-only --cap-drop ALL \
  --security-opt no-new-privileges:true --pids-limit 64 --memory 512m \
  --tmpfs /tmp:rw,noexec,nosuid,nodev,size=64m,mode=1777 \
  --mount "type=bind,src=$workspace,dst=/workspace" --workdir /workspace \
  --env HOME=/tmp/kerna "$image" node -e "$probe"
test -f "$workspace/agent-write.txt"
