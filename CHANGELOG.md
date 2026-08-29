# Changelog

All notable changes to Kerna will be documented in this file.

## [v0.2.9] - 2026-08-29

### Added — govern the client you already use

- **`kerna gateway`**, an MCP server built on the official `rmcp` SDK. Point Qoder,
  Claude Code, Codex or any MCP client at it and every `tools/call` passes through
  the fail-closed policy engine and lands in the event log first. It governs MCP
  tools only; a client's native editor and terminal stay outside that boundary, and
  the docs say so rather than implying otherwise.
- **`kerna contract init`** writes a reviewable `kerna.toml` and a plain-English
  `agent-contract.md`. Nothing in it is model-generated. It enables the demo server
  bundled in the binary, so the policy boundary can be shown on a machine with no
  Docker and no connectors; `--no-demo-server` omits it.
- **`kerna client config` / `kerna client doctor`** print a client configuration and
  then prove it, by starting the gateway and completing a real MCP handshake.
  Printing rather than editing is deliberate: adding ourselves to somebody's client
  config on their behalf is not ours to do.
- **`kerna dashboard`**, loopback-only and CSRF-guarded, reading the SQLite the
  gateway writes. Sessions, tool-call receipts with latencies, approvals,
  containment state and detected hardware, updated live over SSE.
- **A curated local-model catalogue**, pinned to a revision of an MIT-licensed
  registry, which recommends nothing on hardware it has no evidence for.
- **`kerna-observe models`** now offers to fetch the largest model that fits, and
  asks first. `--download` / `--no-download` keep it scriptable.

### Fixed

- **Budgets at the gateway.** A contract could ask for `max_tool_calls = 10` and be
  served fourteen: the gateway had no budget code at all. It now enforces the three
  it can observe — tool calls, wall clock, and bytes returned to the client — and
  deliberately does not claim the three it cannot, since the model runs in the
  customer's own client. Denied and approval-pending calls do not draw on the
  budget; a loud policy must not exhaust the session it protects.
  `kerna_session_status` reports the headroom left.
- **The documented demo now runs.** The gateway demanded Docker before checking
  whether anything was configured, so a fresh contract could not start it. The
  verifier the certification calls a required proof never sent `protocolVersion`
  or `notifications/initialized`, and asserted a server name the gateway did not
  report — clients listed it as `rmcp`, the transport library.
- **`kerna-observe demo` runs on a machine with nothing installed**, which is what
  it always claimed. Three seams imported `httpx` to talk to 127.0.0.1; they use
  the standard library now, and CI runs the demo with no packages so it stays true.
- **The release workflow could not publish.** Its version check read
  `observe.__version__`, which did not exist, so any tag would have died on an
  `AttributeError` instead of reporting a version mismatch.
- Clippy at `-D warnings --all-features`, and a dashboard SSE stream that compared
  a `Value` against a `String` — never equal, so it would have pushed a fresh
  snapshot every second to every open dashboard.

### Upgrading

No configuration changes are required. `runtime_mode = "demo"` is honoured for one
reserved server name and is not a general escape from containment.

## [v0.2.8] - 2026-08-28

### Fixed — upgrade immediately from v0.2.6 or v0.2.7

- **MCP plugins failed to load.** v0.2.6 made `run_command` sandbox a model's commands
  in Docker, which is right. But the default was shared with `McpServerConfig`, so an
  MCP server whose config omitted `runtime_mode` also began launching inside a
  container — one that does not contain the plugin binary. It failed with *"MCP server
  disconnected or returned empty response"*, an obscure message for a configuration
  change nobody made, and the task then died on tool failures.

  This hit any `kerna.toml` written by hand or by a version before the field existed,
  which is most of them. Native is what the rest of the code already assumed:
  `packs.rs`, `plugin_manifest.rs` and both construction sites in `main.rs` all write
  `"native"` explicitly, and only the serde default disagreed.

  **`run_command` still defaults to Docker** — that change was correct and is
  unaffected. Containerising an MCP *plugin* remains available by setting
  `runtime_mode = "docker"` on that server deliberately.

- **CI could not have caught it, and now can.** `cargo audit` ran before `cargo build`
  and `cargo test` in the same job, so while an advisory stood the build and test steps
  never ran at all — for a week, across four commits. The smoke test that catches
  exactly this regression was never reached. The audit is now its own job, ordered
  after build and tests and running whether or not they pass.

## [v0.2.7] - 2026-08-28

### Security

- **`h2` upgraded to 0.4.19** (was 0.4.15), closing
  [RUSTSEC-2026-0258](https://rustsec.org/advisories/RUSTSEC-2026-0258) — "h2 unbounded
  empty DATA frames", a resource-exhaustion denial of service against an HTTP/2 server.
  It reaches Kerna transitively and matters because `kerna serve` and `kerna gateway`
  both serve HTTP.

  **v0.2.6 shipped with this advisory outstanding.** It was published 17 August and CI
  had been failing on it since, but `cargo audit` runs *before* `cargo build` and
  `cargo test` in the CI job, so the red result read as an ordinary broken build and the
  build and test steps had not run on any commit for a week. The release workflow does
  not run the audit at all, so nothing stopped v0.2.6. Anyone on v0.2.6 or earlier
  should upgrade.

## [v0.2.6] - 2026-08-28

### Changed — read this before upgrading

- **`run_command` is sandboxed in Docker by default.** The previous default was
  native: workspace-path confinement, a cleared environment and a timeout. Those are
  real protections and none of them stop a command that stays inside the workspace
  from doing whatever a process there can do — and `run_command` executes commands a
  model wrote and nobody reviewed. Docker adds the boundary the word "sandbox"
  implies, and the image is pinned rather than floating.

  **This affects existing installs.** `kerna init` never wrote `runtime_mode`, so a
  config from v0.2.5 has no value for it and now inherits `docker`. Without the Docker
  CLI, `run_command` fails with a message naming the fix rather than quietly running
  unsandboxed. To keep the old behaviour, set it deliberately:

  ```toml
  runtime_mode = "native"
  ```

### Added

- **`kerna run --audit`** — rung 1. Records every policy decision and enforces none of
  them: nothing is denied, nothing prompts, and each action the policy would have
  stopped is printed and written to the audit trail. Use it to size a policy before
  trusting it. The receipt records what the **policy** decided rather than what was
  enforced, because a trail storing the enforced outcome would report a clean run in
  audit mode and tell you nothing. Enforce remains the default, and audit mode
  announces itself loudly — going unnoticed is its one failure mode.
- **`x-kerna-task` on every model request**, so the runtime's audit trail and a
  sidecar's cost log describe the same turn and can be joined.

### Fixed

- **One policy meaning across both enforcement points.** `docs/POLICY.md` is the
  specification and `docs/policy-conformance.json` is it in executable form; both
  engines run it. Four divergences were found and fixed, including one nobody had
  spotted in review, which the suite failed on its first run.
- **The AgentDojo prevention claim now requires evidence of prevention.**
  `unsafeActionPrevented` was computed as `not injection_task_executed`, so the
  strongest prevention number the benchmark could produce came from a completely
  broken run. It is `true` only when the receipt shows an *enforced* denial, and
  `null` otherwise with `preventionEvidence` naming why.

## [v0.2.5] - 2026-07-25

### Fixed
- `kerna init` now repairs legacy configuration files that contain zero tool
  rounds or retries, or blank runtime and network modes. A refreshed workspace
  can execute its first reviewed task while normal task execution continues to
  preserve user-selected budgets and boundaries.

## [v0.2.4] - 2026-07-24

### Added
- Public, reproducible benchmark evidence for deterministic runtime controls,
  MCP compatibility, restart reliability, and two scoped ToolEmu policy pilots.
- A benchmark results page that explains the test boundaries, examples, raw
  counts, and the difference between a requested action and an action started.

### Fixed
- BFCL compatibility preflight now allows a realistic cold-start window for
  the evaluator's provider dependency graph instead of reporting a false CLI
  failure after thirty seconds.

## [v0.2.3] - 2026-07-15

### Fixed
- Published CLI installers and the npm wrapper now install the reviewed
  curated-plugin bundle alongside the binary. A clean installation can use
  `kerna pack install productivity` without a source checkout or environment
  override.

## [v0.2.2] - 2026-07-15

### Changed
- `kerna trace` now shows each event's recorded policy decision, including
  explicit routine-allowlist denials, alongside the event payload.

## [v0.2.1] - 2026-07-15

### Changed
- Queued approval decisions are now recorded in each task receipt before a
  tool may run, including approved, denied, and expired outcomes. This closes
  the audit trail between a reviewed action and its execution.

## [v0.2.0] - 2026-07-15

### Added
- A local approval queue, task receipts, scoped routine controls, and
  connector-health visibility in the desktop control surface.
- Enforceable MCP manifests, declared-secret handling, recursive trace
  redaction, and fail-closed curated productivity packs.
- An optional Google Calendar OAuth connector: read-only by default, with
  per-action approval and no calendar invitations for event creation.
- A documented initial-cohort acceptance process, including local workflow
  evidence and required disposable-account validation.

### Changed
- CI now builds and tests the desktop control surface on every supported OS,
  and tagged releases attach its native installers alongside the CLI with
  SHA-256 checksum files and reject version/tag mismatches.
- The desktop shell now applies a restrictive local asset and IPC CSP.
- Updated the HTTP client dependency to `reqwest` 0.12; the workspace advisory
  scan has no reported vulnerabilities.

## [v0.1.0] - Initial Public Beta
### Added
- Task Scheduler with autonomous retry loops.
- SQLite Persistent Task and Episodic Memory (`MemoryEngine`).
- Fail-Closed Permission Boundaries (`kerna.toml`).
- Model Context Protocol (MCP) Plugin Architecture.
- Observability commands (`kerna explain`, `kerna export`, `kerna inspect`).
- Secret Leakage Protection via sanitized environment streams.
- Context Overflow Protection for massive tool outputs.
