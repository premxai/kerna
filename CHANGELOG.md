# Changelog

All notable changes to Kerna will be documented in this file.

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
