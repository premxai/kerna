# Changelog

All notable changes to Kerna will be documented in this file.

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
