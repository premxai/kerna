# Demo Readiness

One page for the pitch: what is proven today, the artifact that proves it, and
what is deliberately not claimed. Receipts and test output are the evidence; the
wording in the right-hand column is the wording to say out loud.

Machine-readable gates live in `contracts/claude-pilot-release-gate.json`
(RG-001..RG-005) and `contracts/security-action-inventory.json`
(SB-001..SB-022). `kernel/tests/security_action_inventory.rs` recomputes the
release decision from them, so a claim cannot be made by editing prose.

## What is proven

| Work package | Status | Artifact that proves it | Say this |
|---|---|---|---|
| WP0 client protocol feasibility | Implemented; live proof pending one operator run | `scripts/accept-contained-run.ps1` → `reports/contained-run-proof.json`; byte-boundary parser tests over `kernel/tests/fixtures/anthropic` | "Pinned Claude Code 2.1.270, allow and ask exercised on real client actions" |
| WP1 canonical policy core | Green | `cargo test --locked` (policy conformance + `kernel/src/guard_policy.rs` rules); RG-005 passes | "One typed decision engine; every adapter returns the same decision" |
| WP2 contained supervisor | Containment proven; crash sweep now in code | `scripts/test-claude-container-boundary.ps1` (filesystem, credential, Docker, network probes); `guard_launcher::sweep_stale_sessions` behind `kerna guard cleanup` | "Docker is the boundary. A crashed session's containers and networks are found by label, never by name" |
| WP3 mandatory evidence and approval | Implemented | hash-chained store in `kernel/src/memory.rs`; `wait_for_stream_approval` 300 s one-time binding; interruption pass of the rehearsal records an unreleased action | "Nothing is released until its decision receipt commits, and a released action is never called executed without a result" |
| WP4 local browser workflow | Routes proven over HTTP; UI usable | `/api/v1/dashboard/approvals/:id/approve`, `/workspace`, `/workspace/apply`, `/evidence` driven by the rehearsal; loopback + per-launch CSRF | "Review the diff, apply it into your own clone, export signed evidence, from one browser tab" |
| WP5 packaging (doctor half) | Works on this machine | `kerna guard doctor`, pinned image digest provenance in `guard_launcher` | "Doctor rejects anything that is not the reviewed pinned image" |

## What is not claimed

| Do not say | Evidence-backed wording |
|---|---|
| "Claude Code and Codex" | "Pinned Claude Code today; Codex is a deferred compatibility gate (WP5C)" |
| "Contained on macOS too" | "Containment proven on Windows and Docker Desktop; the physical macOS run is our open release gate RG-003" |
| "Prevents any agent action" | "Every released action is one-time, digest-bound, and non-replayable" |
| "Installable with one command" | "Runs from source today; the release installers ship an earlier tree, so packaging is post-pitch" |
| "We executed N actions" | Only what receipts show: requested, released, result-observed, or outcome-unknown |
| "Budget accounting" | Budgets cover MCP calls, runtime, and output bytes; model spend accounting is out of scope for this milestone |

## Gate state

The rehearsal must be green **before** `RG-004` is flipped to `pass` in
`contracts/claude-pilot-release-gate.json`, with the evidence pointing at
`reports/contained-run-proof.json` and the boundary-probe output. Expected end
state after that run: four of five required gates pass and the decision is still
NO-GO solely on macOS hardware. That is a stronger position than a hand-claimed
GO.

## The evidence database across the boundary

The broker that decides, and the dashboard that reviews, are different machines
in one respect: the broker runs inside the container and the reviewer's
dashboard runs on the host, and only one of them may treat the SQLite file as a
database to write. Two storage rules follow, and both are enforced in
`kernel/src/memory.rs`:

- **never WAL.** WAL keeps its committed-frame index in a `-shm` memory map,
  which is only valid inside one kernel. The host would read an empty queue
  while a real approval was pending.
- **`journal_mode = TRUNCATE`, not DELETE.** A rollback-journal commit ends by
  unlinking the journal file, and that unlink has failed across the Windows
  mount with `Error code 2570: I/O error within xDelete of a VFS object`
  (`SQLITE_IOERR_DELETE`). The broker took the fault, exited, and the contained
  agent reported its own gateway as unreachable. TRUNCATE shrinks the journal to
  zero bytes instead of deleting it, and a zero-length journal is not a hot
  journal, so the host still sees an ordinary committed database.

- **the reviewer never writes.** SQLite's advisory locks are not honored across
  the host/container filesystem translation, so a host dashboard that opens the
  evidence file as a writer races the broker and can revert a transaction the
  broker already committed. `kerna guard` now launches its dashboard with
  `KERNA_DB_READER=1`, and `MemoryEngine::new_reader` opens that handle
  read-only and refuses to bootstrap. A human decision made in the browser is
  handed back to the broker as a one-shot file in `<state>/decisions/`, and the
  broker — the only writer — applies it through the same expiry-, digest-, and
  session-bound checks it uses for a native click. This keeps the release of a
  held action strictly conditional on a receipt the broker itself committed.
- **the reviewer reads what the broker pushes.** The same lock gap cuts the
  other way: a long-lived read-only handle caches pages and is never told to
  discard them, and even reopening the handle per request still served pages
  the mount translation had refreshed not at all — a contained rehearsal once
  showed a real, unexpired held action as an empty approval queue for 240
  seconds. The reviewer's approval queue therefore comes from
  `evidence_view.json`, a small file the broker (the only writer) rewrites with
  a fresh nonce so its size changes and even a caching mount must refetch it,
  timestamped so the reviewer rejects a stale view as the fault it is instead of
  mistaking silence for an empty queue. Publication is not tied to a held
  action: the broker publishes on every hold tick, again the moment it applies a
  decision the reviewer left, once more when an approval expires, and on a
  two-second heartbeat for the life of the server. Without that last one an
  idle broker's view ages past the 15-second freshness window and the dashboard
  answers "no fresh view" while nothing is waiting — which is true but useless,
  because a rehearsal polling the queue reads ten of those as a dead control
  plane and stops.

Kerna reads the mode back from the same `PRAGMA` that sets it and warns on the
trusted side if it did not settle, because journal mode is not recorded in the
database header the way WAL is — opening a second handle and asking proves
nothing about the first. `accept-contained-run.ps1` treats that warning, a
failed receipt commit, and an xDelete fault as run-ending failures rather than
letting them appear as a client-side error.

The approval queue is held to the same standard: `/api/v1/dashboard/approvals`
answers 503 when it cannot read, or when the broker's pushed view is not fresh,
so `{"approvals": []}` always means genuinely nothing is waiting, and never a
storage fault wearing an empty list.

## Running the rehearsal

```powershell
# once: build and prepare (Windows PowerShell 5.1 is enough; pwsh is not required)
$env:CARGO_TARGET_DIR = 'C:\Temp\kerna-pitch-target'   # SSD, never FAT32
cargo build --locked
powershell -ExecutionPolicy Bypass -File scripts/build-claude-agent-image.ps1   # if the preflight says the image is stale

# every rehearsal
powershell -ExecutionPolicy Bypass -File scripts/test-claude-container-boundary.ps1
powershell -ExecutionPolicy Bypass -File scripts/accept-contained-run.ps1 -KeyFile C:\Secure\kerna.key
```

Before spending a provider run, check the harness itself:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/accept-contained-run.ps1 -SelfTest
```

`-SelfTest` starts no container and uses no key. It hand-produces the edit a real
session would produce, then runs the identical review → apply → signed export →
`kerna replay` path, so a broken control-API assumption fails cheaply and
offline. It writes no proof file: a self test is a harness check, never
release-gate evidence.

Current state on this machine: `-SelfTest` passes 5/5 — the workspace read, the
CSRF-and-Origin-bound apply into a separate clean clone, the signed export, and
replay's Ed25519 verification all work over real HTTP. That proves the API
plumbing and the WP4 mechanics. It does **not** prove a contained provider run,
because the edit is hand-made and the evidence chain is empty; only a keyed
rehearsal does that.

`-KeyFile` exists for unattended rehearsals only: the key still travels to the
broker over stdin and is never written to a report, a log, or container
metadata. On stage, type the key at the hidden prompt instead — it is a real
moment in the pitch.

## If live fails

Export one signed bundle from the last green rehearsal. If Docker or the
provider faults on stage, run:

```powershell
kerna replay reports\evidence-approve-<tag>.json
```

Replay verifies the Ed25519 signature and serves the recorded timeline, diff,
and receipts in a visibly marked read-only UI. Say out loud that it is a signed
recording of a run that really happened; the mode is labelled, so the claim
stays honest.

## Deferred by decision

RG-003 physical macOS containment proof · WP5C Codex certification · release
installers, CI-built agent image, code signing and notarization · WP6
design-partner pilot (pilot design ready, partners being selected).
