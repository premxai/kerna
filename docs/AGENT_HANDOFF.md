# Agent Handoff

Last updated: 2026-09-18

## Current repo state

- Workspace root: `F:\Kerna-MVP`
- Kerna implementation repo: `F:\Kerna-MVP\repos\kerna`
- Kerna branch: `mvp/kerna-guard`
- Kerna latest pushed implementation commit before this handoff: `ba1f80e` (`Add native code proposal preflight`)
- Root docs latest local commit: `24a11f6` (`Record native proposal preflight completion`)
- LocalM branch: `mvp/reference-baseline`
- LocalM latest commit: `9570339`
- LocalM is untouched for the native CLI harness work.
- Root docs repo has no configured remote. Do not assume root commits can be pushed.
- Existing user-owned untracked root file: `kerna-workflow-slide.png`. Preserve it.

## Product priority

Continue `PROJECT_STATE.md` `CURRENT_TASK`.

The active work is Native CLI Harness Phase 7: add the first read-only inspection path for
`kerna code`.

Phase 6 is complete and pushed in Kerna at `ba1f80e`:

- `kerna code` emits ordinary assistant prose plus one strict proposal envelope between
  `KERNA_PROPOSAL_JSON_BEGIN` and `KERNA_PROPOSAL_JSON_END`.
- Supported proposed action kinds are `file_read`, `file_write`, `shell`, `network`, and `package`.
- Proposed actions normalize through `ActionIntent` and emit `proposal.preflight`.
- Preflight actions are explicitly non-executable:
  - `executable=false`
  - `receipt_state=preflight_only_not_requested`
- Malformed, missing, unknown, extra-field, and oversized proposal envelopes fail closed.
- No proposal is executed, approved, released, applied, or receipted as requested yet.

## Important boundary rules

- Leave physical macOS verification for later unless the user explicitly changes priority.
- Do not claim Codex certification.
- Do not publish a release without explicit authorization.
- Do not broaden `kerna code` into writes, shell, package installs, network fetches, patch
  application, or original-repository mutation during Phase 7.
- Keep trusted-host Kerna administration separate from model-controlled actions.
- Credit Docker/container containment only when there is runtime proof.
- Preserve unrelated user files and existing LocalM history.

## Phase 7 target

Implement only read-only file inspection for eligible `file_read` proposals from
`proposal.preflight`.

Required behavior:

1. Convert eligible `file_read` proposals into a receipt-bound read action.
2. Record a clear lifecycle:
   - `requested`
   - `released`
   - `result_observed`
   - or fail closed
3. Read only repository files inside the allowed worktree boundary.
4. Block:
   - absolute paths
   - `..` traversal
   - symlink escapes
   - host home access
   - sibling repositories
   - secrets/evidence database paths
   - oversized files
5. Return bounded read results only; do not persist raw customer code/model prose by default.
6. Keep non-`file_read` proposals preflight-only for now.
7. Update SB-021 or add SB-022 so the machine-readable inventory covers the new action surface.

## Good starting files

- `kernel/src/native_code.rs`
  - proposal envelope parsing
  - `ProposalPreflight`
  - `ProposalActionPreflight`
  - Phase 6 tests
- `kernel/src/main.rs`
  - `run_native_code`
  - emits `proposal.preflight`
- `kernel/src/native_cli.rs`
  - stable JSON events and human rendering
- `kernel/src/memory.rs`
  - existing guard receipt event lifecycle
- `kernel/src/server.rs`
  - existing `ActionIntent` + receipt binding helpers for governed actions
- `contracts/security-action-inventory.json`
  - SB-021 native code proposal preflight entry
- `docs/NATIVE_CLI_HARNESS.md`
  - human-readable native harness contract

## Verification baseline from Phase 6

Last verified on Windows before this handoff:

```powershell
$env:CARGO_TARGET_DIR='C:\Temp\kerna-phase6-target'
cargo clippy --locked -- -D warnings
cargo test --locked -- --test-threads=1
cargo build --locked
& 'C:\Temp\kerna-phase6-target\debug\kerna.exe' code 'Plan a safe README update' --repo . --provider mock --json
```

Results:

- strict Clippy passed
- 232 unit/integration tests passed
- 4 inventory tests passed
- build passed
- mock `kerna code` emitted `proposal.preflight`
- inert-key Anthropic probe failed safely and left zero native broker containers/networks
- root `scripts/verify-project.ps1` passed with expected warnings

## Before ending the next checkpoint

1. Run focused tests for changed native code.
2. Run:

   ```powershell
   $env:CARGO_TARGET_DIR='C:\Temp\kerna-phase7-target'
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
