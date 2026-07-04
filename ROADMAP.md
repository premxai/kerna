# Kerna Roadmap

Kerna is transitioning from a proof-of-concept into hardened developer infrastructure. We expect rapid evolution of the internal APIs before stabilizing at `v1.0.0`.

## v0.1.0 (Current)
- Core Agent Runtime
- SQLite Memory Engine
- MCP Plugin Support
- Observability (Inspect, Explain, Export)

## v0.2.0 (The Observability Update)
- **Event Bus Architecture**: Decoupling the scheduler loop using a Pub/Sub event bus.
- **`kerna trace`**: A dedicated timeline renderer that replays exact state transitions of tasks in real-time.
- **Metrics API**: Prometheus-compatible endpoints to track token usage, cost, and latency across swarms.

## v0.3.0 (The Ecosystem Update)
- **Stable Plugin SDK**: Official Rust, Python, and TypeScript SDKs for building native plugins outside the raw JSON-RPC MCP standard.
- **Plugin Registry**: `kerna plugins install github.com/user/repo`
- **Deterministic Mode**: `--deterministic` flag to force seed matching and temperature 0 across all tools.

## v1.0.0 (The Cloud Update)
- **Cloud Sync**: Securely sync your SQLite memory databases across machines.
- **Team Workspaces & Shared Memory**: Organizations can inject group context into agent loops.
- **Distributed Runtime**: Offload tasks to remote Kerna worker nodes securely.
