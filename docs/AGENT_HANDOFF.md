# Agent Handoff

Last updated: 2026-09-18

## Current repo state

- Workspace root: `F:\Kerna-MVP`
- Kerna implementation repo: `F:\Kerna-MVP\repos\kerna`
- Kerna branch: `mvp/kerna-guard`
- Kerna latest pushed implementation commit before this handoff: `0263a60` (`Add native code contained read-only inspection`)
- Root docs latest local commit: `24a11f6` (`Record native proposal preflight completion`)
- LocalM branch: `mvp/reference-baseline`
- LocalM latest commit: `9570339`
- LocalM is untouched for the native CLI harness work.
- Root docs repo has no configured remote. Do not assume root commits can be pushed.
- Existing user-owned untracked root file: `kerna-workflow-slide.png`. Preserve it.

## Product priority

Continue `PROJECT_STATE.md` `CURRENT_TASK`.

The active work is Native CLI Harness Phase 8: add the first approval surface for native
`kerna code`, so `ask`-policy inspection reads can be released only through an explicit one-time
receipt-bound decision.

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
