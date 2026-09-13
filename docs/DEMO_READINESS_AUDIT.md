# September 14 Demo Readiness Audit

**Decision:** green for the guided local/Wasmer demo; amber for provider-dependent and publishing
steps. This is not a claim of production readiness.

## Verified now

- The product front door is `kerna`; setup is `kerna doctor`. `kerna claude`, `kerna sandbox`,
  `kerna replay`, `kerna skills`, and `kerna dashboard` are the only demo commands that need to be
  remembered. Legacy commands remain behind `kerna advanced`.
- The full Rust suite passes: 195 tests, 0 failures, serial execution, local SSD Cargo target.
- The live Wasmer runtime completes a Python calculation and denies host-filesystem and outbound-
  network access. The governed MCP path stores only capability, package, timing, status, and
  output digest evidence.
- Routing is deterministic and sticky. Explicit local mode fails closed if Ollama is unavailable;
  it never silently retries in cloud.
- A cloud-primary request and its local shadow use the same request digest. The shadow request has
  tools removed and cannot release actions. Primary and shadow prose is not stored.
- Exact, expiring, one-time approvals remain bound to session, task, agent version, protocol,
  policy digest, worktree baseline, and canonical action digest.
- The dashboard is loopback-only, mutation endpoints require exact-origin CSRF, signed replay is
  read-only, and the 10-day fixture is visibly marked `SIMULATED` and not customer telemetry.
- Windows and macOS/Linux source-checkout installers parse successfully. Expensive caches, models,
  runtime packages, sessions, and replay evidence are directed away from the FAT32 source drive.

## Required operator checks before the stage demo

1. Run `kerna doctor` after connecting to venue power/network.
2. Run one local prompt and one Wasmer success/failure sequence.
3. Enter the Anthropic key only in Kerna's hidden cloud-launch prompt, then complete one real
   cloud-primary/local-shadow request. This provider-backed variation cannot be certified without
   the operator's key.
4. Export and replay the signed rehearsal bundle once after the final run.

## Honest amber items

- **Anthropic live variation:** code and deterministic tests are green; a fresh cloud-primary run
  still requires the operator's API key.
- **Tenki live execution:** adapter and failure cleanup are implemented, but live authentication is
  unavailable until a Tenki key is supplied. Keep Tenki outside the critical demo path.
- **Public distribution:** GitHub and npm currently publish v0.2.5. The short CLI, shadow telemetry,
  and sponsor-runtime changes are not public until a newer signed release is cut.
- **macOS/Linux:** the installer has syntax/path validation on Windows but has not been exercised on
  a real macOS host in this audit.
- **Claude containment:** the model seam and delegated MCP tools are governed, while the Claude host
  process itself is not fully containerized. Native Claude tools that bypass MCP are not claimed as
  governed.
- **Comparison scope:** live comparison covers provider/model, status, timing, bytes, tool authority,
  and output digests. It does not score semantic quality because model prose is deliberately not
  retained. The 10-day quality/routing story is simulated rehearsal data.
- **Budgets and orchestration:** tool-call/runtime/output limits and session supervision are active;
  full model spend accounting and broad multi-agent orchestration are not part of this demo.

## Install commands

Windows checkout:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\install-hackathon.ps1
kerna doctor
kerna
```

macOS, Linux, or a remote shell checkout:

```sh
sh ./scripts/install-hackathon.sh
kerna doctor
kerna
```

The existing public installers and `npm install -g @premxai/kerna` install v0.2.5. Publish and
verify a newer signed release before advertising those paths for this demo build.

## Dashboard design rationale

The dashboard uses progressive disclosure: route and machine readiness first, then live runtime
comparison and approvals, followed by historical context and evidence. This follows the useful
operator-console patterns seen in [Preloop](https://github.com/Preloop/Preloop),
[Hecate](https://github.com/hecatehq/hecate), and
[Ferro AI Gateway](https://github.com/ferro-labs/ai-gateway), while keeping Kerna's controls and
security boundary explicit rather than copying their product claims.
