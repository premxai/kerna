#!/usr/bin/env bash
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
image="kerna-claude-agent:0.2.9-claude-2.1.270"
docker build --pull=false --file "$repo/docker/claude-agent/Dockerfile" --tag "$image" "$repo"
inspection="$(docker image inspect "$image" --format '{{.Id}}|{{index .Config.Labels "dev.kerna.contract"}}|{{index .Config.Labels "dev.kerna.claude-code-version"}}|{{index .Config.Labels "dev.kerna.claude-package"}}|{{index .Config.Labels "dev.kerna.claude-package-integrity"}}|{{index .Config.Labels "dev.kerna.rust-base-digest"}}|{{index .Config.Labels "dev.kerna.node-base-digest"}}')"
expected_suffix='|claude-agent-v1|2.1.270|@anthropic-ai/claude-code@2.1.270|sha512-0zMkfIWQu7/SG56VP8r780HZWvrNShzK28AbAnhKRK0ns+ToGXPT0W8UqyZmZCUKAkJDd5//TrwSOhk1+hysiw==|sha256:af306cfa71d987911a781c37b59d7d67d934f49684058f96cf72079c3626bfe0|sha256:8a34c4ab3ea2c5cd194f07e317b2a8f09461d3c8b05c4e34c8ccd56d56024c4d'
case "$inspection" in
  sha256:????????????????????????????????????????????????????????????????"$expected_suffix") ;;
  *) echo "Claude agent image provenance verification failed" >&2; exit 1 ;;
esac
printf '%s provenance=verified\n' "${inspection%%|*}"
