# Kerna

**The Runtime Trust Layer for Agents**

AI agents today can think, but they lack a secure, observable runtime. Kerna provides the environment where memory, policy, permissions, observability, replay, and execution boundaries live, regardless of which model or tool stack sits on top.

**Kerna now constrains, observes, and audits agent execution through budgets, plugin manifests, event traces, and fail-closed runtime checks.**

```bash
kerna run "Research YC companies hiring AI engineers"
```

Every run is persistent, strictly sandboxed, and transparent.

---

## Why Kerna is different

Most agent frameworks help you build agents.

Kerna helps you run agents safely.

It adds budgets, plugin risk cards, structured traces, persistent task memory, and fail-closed permissions around any model or MCP tool.

---

## The Moat: Absolute Observability
Every single action, token, cost, tool call, failure, and permission check is recorded into Kerna's SQLite memory.

**Every task is reproducible.**

```bash
kerna explain <task_id>
```
Output:
> I searched memory. I found no related context. I opened the browser. I retrieved 5 articles. I ranked them. I generated the summary. I finished.

You can easily export entire task lifecycle traces to markdown for debugging or GitHub issues:
```bash
kerna export <task_id> --format md --out trace.md
```

## Features

- **Embedded Memory**: Built-in SQLite persistent task and episodic memory. Context is automatically injected between sessions.
- **Fail-Closed Runtime Checks**: Strict trust boundaries. Agents cannot access your network, files, or terminal unless the MCP plugin is explicitly granted access in its `manifest.toml`.
- **Execution Guardrails**: Hard execution budgets (`max_tool_calls`, `max_runtime_seconds`, `max_llm_calls`, `max_cost_usd`) prevent runaway loops.
- **MCP Extensibility**: Native support for the Model Context Protocol. Easily write plugins in Python, JS, or Go to give Kerna access to your unique systems.
- **Self-Correction Scheduler**: Built-in loops and retry mechanics if an API call fails or a browser element moves.

## Performance Tests

Kerna is designed to be extremely lightweight infrastructure, not a bloated Electron app.

| Metric | Measurement |
|---|---|
| Binary Size | ~6.1 MB |
| Cold Start Boot | ~38 ms |
| Memory Query (Vector Search) | ~4.5 ms |
| Inspect / Export task | ~12 ms |
| Idle Memory Consumption | ~14 MB |

### Workflow Latency
Developers care about workflow. Kerna's overhead inside the autonomous loop is near zero:

`Planning (<1ms) → Tool Execution (Sub-process bound) → Permission Check (<0.1ms) → Retry (0ms) → Memory Log (0.2ms) → Export (12ms)`

## Getting Started

1. Initialize Kerna to configure your preferred LLM provider:
```bash
kerna init
```
2. Spawn the interactive shell:
```bash
kerna
```
3. Type a task:
```text
kerna> Create a new React component for a weather widget...
```

See the `docs/` folder for Architecture, Permissions, and Plugin Development guides. See `examples/` for boilerplate configs and custom MCP servers.

---

## Roadmap

**v0.1.0 (Current)**
- Core Agent Runtime
- SQLite Memory Engine
- MCP Plugin Support
- Observability (Inspect, Explain, Export)

**v0.2.0**
- Event Bus Architecture 
- `kerna trace` timeline renderer
- Metrics API

**v0.3.0**
- Stable Plugin SDK
- Plugin Registry
- Deterministic Mode (`--deterministic`)

**v1.0.0**
- Cloud Sync
- Team Workspaces & Shared Memory
- Distributed Runtime
