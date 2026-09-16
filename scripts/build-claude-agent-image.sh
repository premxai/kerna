#!/usr/bin/env bash
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
image="kerna-claude-agent:0.2.9-claude-2.1.270"
docker build --pull=false --file "$repo/docker/claude-agent/Dockerfile" --tag "$image" "$repo"
docker image inspect "$image" --format '{{.Id}} {{index .Config.Labels "dev.kerna.contract"}} {{index .Config.Labels "dev.kerna.claude-code-version"}}'
