# Agent Handoff

Last updated: 2026-09-19

## Current repo state

- Workspace root: `F:\Kerna-MVP`
- Kerna implementation repo: `F:\Kerna-MVP\repos\kerna`
- Kerna branch: `mvp/kerna-guard`, pushed through `c3278eb`
  (`Record the contained run as proof and correct the empty-queue diagnosis`)
- Root docs latest local commit: see `SESSION_LOG.md` at `F:\Kerna-MVP` (root has no remote)
- LocalM branch: `mvp/reference-baseline`
- LocalM latest commit: `9570339`
- LocalM is untouched for the native CLI harness work.
- Root docs repo has no configured remote. Do not assume root commits can be pushed.
- Existing user-owned untracked root file: `kerna-workflow-slide.png`. Preserve it, along with the
  user-owned untracked `website-next/` in this repo.

## Product priority

Pitch readiness is delivered. The provider-backed contained rehearsal ran green on Windows with a
real Anthropic key, and `RG-004` was flipped only after
`reports/contained-run-proof.json` existed (`recorded_at 2026-09-19T08:06:36Z`). The release
decision still computes NO-GO, on `RG-003` physical macOS containment alone — four proven gates and
one named hardware limitation. `kernel/tests/security_action_inventory.rs` recomputes that decision
from `contracts/claude-pilot-release-gate.json`, so a gate cannot be claimed by editing prose.

Current priority is `docs/DEMO_READINESS.md`: say exactly what its *What is proven* table supports
and nothing from the *What is not claimed* column.

After the pitch, the thread to resume is Native CLI Harness Phase 8 (an approval surface for native
`kerna code` inspection reads), described below.

To reproduce the contained proof without a key — the three passes need one, everything else does
not:

```powershell
$env:CARGO_TARGET_DIR='C:\Temp\kerna-pitch-target'
cargo build --locked
powershell -ExecutionPolicy Bypass -File scripts/accept-contained-run.ps1 -SelfTest
powershell -ExecutionPolicy Bypass -File scripts/test-claude-container-boundary.ps1
& "$env:CARGO_TARGET_DIR\debug\kerna.exe" guard doctor
```

Rehearsal mechanics worth knowing before scheduling another keyed run:

- The harness refuses to start while any `dev.kerna.managed=true` container is live, and two
  rehearsals cannot share a machine anyway (the Docker VM budgets ~8 GiB to containers capped at
  2 GiB each, and all three passes advertise dashboard port 8877).
- It validates that the agent image is newer than `kernel/src`, so any source edit requires
  `scripts/build-claude-agent-image.*` before the next keyed run. The doc-comment-only edit in
  `c3278eb` is why the image on this machine currently looks stale.
- `-KeyFile` takes a path outside any repository. Never paste a key into a chat, a log, or a
  commit; the operator deletes the file when the work ends, as happened here.
- PowerShell 5.1 traps now encoded in the harness: `@($response).field` binds a bare object when the
  array has one element and that object has no `.Count`, so a single held approval reads as an empty
  queue. Write `@($response.field)`. And `"{0:…} $msg" -f $msg` throws `FormatError` when the
  interpolated text contains braces — concatenate instead when logging a caught exception.

Phase 7 is complete and pushed in Kerna at `0263a60`:

- Eligible `file_read` proposal actions convert into receipt-bound, contained, read-only inspection.
- Every other action kind stays preflight-only and non-executable.
- Inspection lifecycle events: `inspection.requested`, `inspection.blocked`, `inspection.released`,
  `inspection.result_observed`, `inspection.outcome_unknown`.
- The guard binding commits `requested` plus `released` before any read happens; the result is
  recorded `result_observed` with digest-only details (bytes, SHA-256, truncated flag).
- Read failures after release mark the receipt `outcome_unknown`, never executed.
- Targets fail closed on absolute/drive/home/`..`/device-name paths, `.git` internals, secret
  paths, evidence-database sidecars, symlink escapes, missing, non-regular, and >512 KiB files.
- Reads return an 8 KiB preview plus full-content SHA-256; file content is never persisted.
- Containment label is `trusted_cli_worktree_read`; no container containment is claimed.
- Policy `deny` blocks; policy `ask` stays preflight-only because Phase 7 has no approval surface.
- SB-022 classifies the new surface; SB-021 now defers file-content reads to it.

## Important boundary rules

- Leave physical macOS verification for later unless the user explicitly changes priority.
- Do not claim Codex certification.
- Do not publish a release without explicit authorization.
- Do not broaden `kerna code` into writes, shell, package installs, network fetches, patch
  application, or original-repository mutation during Phase 8.
- Keep trusted-host Kerna administration separate from model-controlled actions.
- Credit Docker/container containment only when there is runtime proof.
- Preserve unrelated user files and existing LocalM history.

## Phase 8 target

Implement only the approval surface for contained read-only inspection. The command remains
read-only: no writes, shell, package, network, patch apply, or original-repository mutation.

Required behavior:

1. `ask`-policy `file_read` inspection requests create a pending approval bound to the existing
   `GuardActionBinding` machinery (session, agent version, policy digest, worktree baseline,
   canonical action digest, expiry).
2. The approval decision happens in the trusted CLI process (for example a terminal
   `dialoguer` confirm) — never model-controlled.
3. Release only after the decision receipt commits; approvals are one-time.
4. Denial, expiry, mismatch, or persistence failure leaves the read blocked and unreleased.
5. `allow`-policy reads keep the existing auto-release path; `deny` stays blocked.
6. Keep the `inspection.*` event vocabulary; add approval-shaped events only if needed.
7. Update SB-022 (or add SB-023) so the machine-readable inventory covers the approval surface.

## Good starting files

- `kernel/src/native_inspect.rs`
  - inspection boundary, receipt lifecycle, and tests
- `kernel/src/native_code.rs`
  - proposal envelope parsing and parsed actions
- `kernel/src/main.rs`
  - `run_native_code` inspection orchestration
- `kernel/src/native_cli.rs`
  - stable JSON events and human rendering
- `kernel/src/memory.rs`
  - guard receipts, approvals, `create_guard_action` ask path
- `kernel/src/server.rs`
  - existing approval binding and one-time release proofs
- `contracts/security-action-inventory.json`
  - SB-022 native inspection entry
- `docs/NATIVE_CLI_HARNESS.md`
  - human-readable native harness contract

## Verification baseline from the pitch-readiness checkpoint

Verified on Windows, Docker Desktop running, no provider key available to the agent:

```powershell
$env:CARGO_TARGET_DIR='C:\Temp\kerna-pitch-target'
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked -- --test-threads=1
cargo test --locked --test security_action_inventory -- --test-threads=1
cargo build --locked
powershell -ExecutionPolicy Bypass -File scripts/test-claude-container-boundary.ps1
powershell -ExecutionPolicy Bypass -File scripts/accept-contained-run.ps1 -SelfTest
& "$env:CARGO_TARGET_DIR\debug\kerna.exe" guard doctor
```

Results:

- strict Clippy (including test targets) passed; 251 unit tests and all 4 inventory tests passed
- the boundary probe reported `workspace=true, broker=true` and
  `evidence_store/host_home/docker_socket/provider_key/browser_key/unrelated_plugin_key/public_network=false`
  at `uid=10001`
- the rehearsal self test passed 5/5: uncommitted-diff detection, CSRF-and-Origin-bound apply into a
  separate clean clone, the edit present in that clone, signed bundle export, and `kerna replay`
  verifying the Ed25519 signature
- `guard doctor` exited 0 and reported the pinned contained image
  `sha256:35948e59cf7f` plus the new stale managed-resource check
- an inert-key `kerna claude --route cloud` launch reached the provider and stopped at a redacted
  `401`, which proves the key-file read, the launch sweep, the broker egress path, and the
  dashboard/CSRF advertisement without spending credit

## Verification baseline from the keyed contained run

The same commands on the same machine, plus one provider-backed rehearsal at `c3278eb`, produced
`reports/contained-run-proof.json` (`recorded_at 2026-09-19T08:06:36Z`, 11/11 assertions).
Re-verified on that tree: strict Clippy on all targets, 255 unit tests, 4 inventory tests,
`cargo fmt --all -- --check`, `cargo build --locked`, and a fresh boundary-probe read.

- accept pass: two held `Bash` actions approved across the boundary, released, and recorded
  `result_observed`; one `Edit` auto-allowed; the `src/add.rs` diff reviewed in the dashboard and
  applied over the control API into a separate clean clone; the signed bundle
  (`94760575576023FD…57FA`) re-verified by `kerna replay`.
- reject pass: one held action denied, receipt `blocked`, apply refused on an empty selection, the
  target still at its stub body.
- interrupt pass: one held action left undecided while the host process tree was killed; the receipt
  stayed `pending`, no ask action was released, the orphan containers and networks were located by
  the `dev.kerna.managed` label, and cleanup left zero managed containers.
- the artifact records `provider_keys_injected: false` and stores only paths, digests, receipt
  states, and `git apply` statistics.

Known limits carried forward, none of which weaken the above: contained receipts show
`agent_version: "unknown"` because the pin is enforced at launch rather than reported per action;
`Open-ReviewDashboard` still opens the retained database with a writer handle although decisions
travel through the one-shot spool; and dashboard panels other than the approval queue still read
across the containment mount.

Two Windows PowerShell traps worth remembering, both now handled by helpers in
`scripts/accept-contained-run.ps1`:

- `Invoke-WebRequest`.Content rewrites embedded newlines to CRLF, so a signed evidence bundle saved
  from it no longer matches the signed bytes and replay fails with a signature error. The harness
  downloads bytes with `System.Net.WebClient.DownloadData` instead.
- `Set-Content -Encoding UTF8` writes a byte-order mark, and `serde_json` rejects it as
  `expected value at line 1 column 1`. JSON artifacts are written BOM-free.

## Verification baseline from Phase 7

Last verified on Windows before this handoff:

```powershell
$env:CARGO_TARGET_DIR='C:\Temp\kerna-phase7-target'
cargo clippy --locked -- -D warnings
cargo test --locked -- --test-threads=1
cargo test --locked --test security_action_inventory -- --test-threads=1
cargo build --locked
& 'C:\Temp\kerna-phase7-target\debug\kerna.exe' code 'Plan a safe README update' --repo . --provider mock --json
```

Results:

- strict Clippy passed
- 248 unit/integration tests passed
- 4 inventory tests passed
- build passed
- mock `kerna code` emitted the full inspection lifecycle
  (`inspection.requested` → `inspection.released` → `inspection.result_observed`)
- the mock smoke receipt chain in `kerna.db` was `requested` → `released` → `result_observed`
  with digest-only payload and no file content
- inert-key Anthropic probe failed safely and left zero native broker containers/networks
- root `scripts/verify-project.ps1` passed with expected warnings
- pre-existing non-native gateway Docker resources (`kerna-broker-*`, `kerna-agent-*`,
  `kerna-egress-*` networks) predate this work; they are not created by the native path and
  were left untouched

## Before ending the next checkpoint

1. Run focused tests for changed native code.
2. Run:

   ```powershell
   $env:CARGO_TARGET_DIR='C:\Temp\kerna-phase8-target'
   cargo clippy --locked -- -D warnings
   cargo test --locked -- --test-threads=1
   cargo test --locked --test security_action_inventory -- --test-threads=1
   cargo build --locked
   ```

3. Run at least one `kerna code --provider mock --json` smoke.
4. Run the inert-key broker cleanup probe if broker code or native provider lifecycle changes.
5. Update `PROJECT_STATE.md`, `SESSION_LOG.md`, and the Kerna docs/inventory.
6. Run `scripts/verify-project.ps1` from `F:\Kerna-MVP`.
7. Commit coherent Kerna changes and push `mvp/kerna-guard`.
8. Commit root docs locally. Root has no remote unless a future agent adds one intentionally.
