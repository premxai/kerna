# Kerna Claude-First Demo

This is a guided local demo. Kerna owns the model-route decision, policy, approvals, and signed
evidence. Wasmer and Docker isolate delegated execution. The Claude host process runs from a
disposable clone and is **not** fully containerized; the dashboard repeats that boundary.

## One-time setup

Run from the Kerna checkout on Windows. All large artifacts remain on the local SSD.

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\bootstrap-demo.ps1
C:\Temp\kerna-target\debug\kerna.exe guard doctor --demo --repo .
```

The bootstrap installs the pinned Node bridge at `C:\KernaData\kerna-demo`, configures Ollama
models at `C:\KernaData\ollama-models`, pulls `qwen2.5-coder:7b`, prewarms Claude Code 2.1.270,
and leaves Cargo/session data on `C:`.

## Guided sequence

1. Run `kerna guard doctor --demo --repo .`. Show the system card: GPU/VRAM, Ollama, local model,
   Claude Code, Wasmer, Docker, and amber Tenki authentication state.
2. Prove a local-only session with a read-only task:

   ```powershell
   kerna guard claude --repo . --route local --prompt "Summarize the security invariants. Do not modify files."
   ```

   The dashboard prints the disposable clone path and its chosen loopback URL. Show the sticky
   `ollama / qwen2.5-coder:7b` route and zero cloud shadow events.
3. Prove automatic cloud routing and local shadow with a multi-file edit task:

   ```powershell
   kerna guard claude --repo . --route auto --shadow
   ```

   At the hidden prompt, paste the Anthropic API key. It is sent only to the trusted broker stdin,
   never to Claude, command arguments, project files, receipts, Docker metadata, or exports. Use a
   task such as: “Fix the fixture typo, run the focused test, and explain the change.” The route is
   recorded before contacting a provider; the local shadow receives no tools and cannot release an
   action.
4. In the dashboard, show one allowed MCP call, an approval-held `secret_probe`, and denied
   `network_probe`. Approvals are one-time and bound to the exact session/action/policy/worktree
   receipt. Native Claude tools outside the MCP path are not claimed as covered.
5. Ask Claude to call `kerna_sandbox_run` with Wasmer, or use the reliable fallback:

   ```powershell
   kerna demo sandbox
   kerna demo sandbox --code "print(open('C:/Windows/System32/drivers/etc/hosts').read())"
   kerna demo sandbox --code "import urllib.request; print(urllib.request.urlopen('https://example.com').read())"
   ```

   The first prints `285`. The second and third fail safely: the guest sees no host filesystem and
   has no DNS/network capability. Kerna stores only package/capability metadata, outcome, timing,
   and an output digest—never sandbox source or raw output.
6. Export signed evidence from the dashboard. If connectivity fails, replay the exact bundle:

   ```powershell
   kerna demo replay C:\KernaData\kerna-demo\recorded-rehearsal.json
   ```

   Replay verifies Ed25519 before serving, adds a `RECORDED REHEARSAL` watermark, and disables every
   mutation/approval control.

## Tenki

Tenki is intentionally outside the live critical path. Until its API key is supplied through the
hidden `kerna demo sandbox --backend tenki` prompt, doctor and the dashboard must say
`configured — authentication required`. Kerna explicitly asks Tenki for both inbound and outbound
networking to be disabled and closes the admitted VM in `finally`.

## Honest claim

Say: “Kerna decides what is allowed, where the model request goes, and what gets recorded. Wasmer
and Docker isolate delegated code. We do not claim that the Claude host process itself is already
containerized, and we do not claim native client tools are governed by the MCP gateway.”
