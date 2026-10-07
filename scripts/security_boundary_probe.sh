#!/usr/bin/env bash
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/tmp/kerna-security-boundary}"
cd "$repo/kernel"

probe() {
  echo "[probe] $1"
  cargo test --locked "$1" -- --exact --nocapture
}

probe guard_protocol::tests::malformed_arguments_fail_closed
probe folders::tests::safe_join_rejects_symlink_escape
probe trust_layer_validation::test_declared_secret_reaches_plugin_undeclared_does_not
probe trust_layer_validation::test_queued_approval_is_recorded_before_tool_execution
probe gateway::tests::contained_filesystem_fixture_reads_and_approval_gates_real_writes
probe server::tests::demo_sandbox_denials_have_auditable_receipts_on_both_routes
probe sponsor_runtime::tests::kerna_denies_escape_intent_before_backend_contact
probe sandbox::tests::docker_mode_without_docker_explains_itself
cargo test --locked --test security_action_inventory

echo "Security boundary probes passed. All payloads were inert test fixtures."
