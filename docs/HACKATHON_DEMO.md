# Kerna Claude-First Demo

This is a guided local demo. Kerna owns the model-route decision, policy, approvals, and signed
evidence. Wasmer and Docker isolate delegated execution. The Claude host process runs from a
disposable clone and is **not** fully containerized; the dashboard repeats that boundary.

## Shortest path

```text
kerna init --demo  # first run asks; later runs launch the saved non-secret profile
kerna init --demo --brief  # same launch with compact terminal output
kerna doctor   # scan hardware, runtimes, models, storage, and provider readiness
kerna doctor --brief  # one-line readiness summary for the stage
kerna          # start governed Claude with automatic routing and local shadow
```

The longer `kerna guard ...` and `kerna demo ...` forms remain as compatibility aliases, but they
are hidden from the primary help. Useful explicit overrides are `kerna claude --route local`,
`kerna claude --route cloud`, `kerna sandbox`, and `kerna replay <evidence.json>`.

## Windows setup

Run from the Kerna checkout on Windows. All large artifacts remain on the local SSD.

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\install-hackathon.ps1
kerna init --demo
kerna doctor
```

The bootstrap installs the pinned Node bridge at `C:\KernaData\kerna-demo`, configures Ollama
models at `C:\KernaData\ollama-models`, pulls `qwen3.5:9b`, prewarms Claude Code 2.1.270,
and leaves Cargo/session data on `C:`. `kerna init --demo` records only the selected non-secret
profile, checks Docker/Wasmer/Tenki readiness, and opens the dashboard when ready. Provider keys
are requested later by the operation that uses them.

## Guided sequence

### 90-second judging cut

Complete installation, model pulls, Docker startup, and hidden provider-key entry before the
timer. On stage, use the already-saved profile:

```powershell
kerna init --demo
```

Then keep the dashboard on Route now. In one pass: show cloud primary plus local shadow, show the
same short read-only task routed locally, trigger one allowed and one blocked governed tool, run
Wasmer's `285` computation, and finish on the receipt hash, simulated ten-day chart, and signed
evidence button. Use `kerna sandbox --backend tenki` only if the remote key and VM are already
known-good; otherwise keep the truthful amber readiness card and use recorded replay.

1. Run `kerna init --demo` once, then `kerna doctor`. Show the routing policy first, then the system card: GPU/VRAM, Ollama, local model,
   Claude Code, Wasmer, Docker, and amber Tenki authentication state.
2. Prove a local-only session with a read-only task:

   ```powershell
   kerna claude --route local --prompt "Summarize the security invariants. Do not modify files."
   ```

   The dashboard prints the disposable clone path and its chosen loopback URL. Show the sticky
   `ollama / qwen3.5:9b` route and zero cloud shadow events.
3. Prove automatic cloud routing and local shadow with a multi-file edit task:

   ```powershell
   kerna
   ```

   At the hidden prompt, paste the Anthropic API key. It is sent only to the trusted broker stdin,
   never to Claude, command arguments, project files, receipts, Docker metadata, or exports. Use a
   task such as: “Fix the fixture typo, run the focused test, and explain the change.” The route is
   recorded before contacting a provider; the local shadow receives no tools and cannot release an
   action. The dashboard pairs primary and shadow by the same request digest, then compares status,
   duration, output size, output digest, and tool authority. It does not claim a live semantic score
   because raw model prose is not retained.
4. In the dashboard, show one allowed MCP call, an approval-held `secret_probe`, and denied
   `network_probe`. Approvals are one-time and bound to the exact session/action/policy/worktree
   receipt. Native Claude tools outside the MCP path are not claimed as covered.
5. Ask Claude to call `kerna_sandbox_run` with Wasmer, or use the reliable fallback:

   ```powershell
   kerna sandbox
   kerna sandbox --code "print(open('C:/Windows/System32/drivers/etc/hosts').read())"
   kerna sandbox --code "import urllib.request; print(urllib.request.urlopen('https://example.com').read())"
   ```

   The first prints `285`. The second and third fail safely: the guest sees no host filesystem and
   has no DNS/network capability. Kerna stores only package/capability metadata, outcome, timing,
   and an output digest—never sandbox source or raw output.
6. Export signed evidence from the dashboard. If connectivity fails, replay the exact bundle:

   ```powershell
   kerna replay C:\KernaData\kerna-demo\recorded-rehearsal.json
   ```

   Replay verifies Ed25519 before serving, adds a `RECORDED REHEARSAL` watermark, and disables every
   mutation/approval control.

## Tenki

Tenki is a required live segment for this rehearsal. Until its API key is supplied through the
hidden `kerna sandbox --backend tenki` prompt, doctor and the dashboard must say
`configured — authentication required`. Kerna explicitly asks Tenki for both inbound and outbound
networking to be disabled and closes the admitted VM in `finally`. If Tenki authentication fails,
use the signed replay bundle and keep Wasmer as the live containment proof; do not claim a successful
Tenki run that did not happen.

## CI proof

The existing GitHub workflow remains backstage proof. On Windows it runs
`scripts/verify-demo.ps1`, which checks the built CLI/help surfaces and packaging bridge without
provider keys or customer prompts, then uploads `reports/demo-readiness/ci.json`. Only the commit,
checks, timestamps, and pass/fail status belong in the evidence story.

## Install and distribution status

- **Windows demo checkout:** `powershell -ExecutionPolicy Bypass -File .\scripts\install-hackathon.ps1`
  installs requirements, builds on `C:`, places `kerna.exe` in `C:\KernaData\bin`, and updates the
  user PATH.
- **macOS/Linux/remote checkout:** source installers remain available but are outside this Windows
  venue rehearsal and are not a live support claim for this event.
- **Published stable release:** the existing `install.ps1`, `install.sh`, npm launcher, and GitHub
  release workflow are real, but the public `v0.2.5` artifacts predate this hackathon branch. Do not
  tell judges the short CLI or shadow comparison is in the public release until a new signed tag is
  built and published.

## Dashboard data labels

The live routing, readiness, approvals, receipts, containment, and primary/shadow records are real.
The 10-day routing chart is a deterministic illustrative fixture and is visibly labeled
`SIMULATED DATA`; it is not customer telemetry, a benchmark, or a quality claim.

## Honest claim

Say: “Kerna decides what is allowed, where the model request goes, and what gets recorded. Wasmer
and Docker isolate delegated code. We do not claim that the Claude host process itself is already
containerized, and we do not claim native client tools are governed by the MCP gateway.”
