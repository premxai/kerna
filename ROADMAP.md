# Kerna Roadmap

Kerna is transitioning from a proof-of-concept into hardened developer infrastructure. We expect rapid evolution of the internal APIs before stabilizing at `v1.0.0`.

## v0.2.1 (Current: controlled productivity cohort)
- Local-first notes, calendar, weather, web research, and optional Google Calendar.
- Fail-closed MCP manifests, explicit approvals, scoped read-only routines, and task receipts.
- Local desktop control surface for tasks, connector health, routines, and approvals.
- Cohort release artifacts with version guards and SHA-256 checksums.

## v0.3.0 (Next)
- **Event Bus Architecture**: Decoupling the scheduler loop using a Pub/Sub event bus.
- **Metrics API**: Prometheus-compatible endpoints to track token usage, cost, and latency across swarms.
- **Broader curated connectors**: OAuth-backed email, documents, and collaboration tools after separate acceptance testing.

## v0.3.0 (The Ecosystem Update)
- **Stable Plugin SDK**: Official Rust, Python, and TypeScript SDKs for building native plugins outside the raw JSON-RPC MCP standard.
- **Plugin Registry**: `kerna plugins install github.com/user/repo`
- **Deterministic Mode**: `--deterministic` flag to force seed matching and temperature 0 across all tools.

## v1.0.0 (The Cloud Update)
- **Cloud Sync**: Sync your SQLite memory databases across machines.
- **Team Workspaces & Shared Memory**: Organizations can inject group context into agent loops.
- **Distributed Runtime**: Offload tasks to remote Kerna worker nodes.
