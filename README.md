# Kerna

**The Runtime Trust Layer for Agents**

AI agents today can think, but they lack a secure, observable runtime. Kerna provides the environment where memory, policy, permissions, observability, replay, and execution boundaries live, regardless of which model or tool stack sits on top.

**Kerna now constrains, observes, and audits agent execution through budgets, plugin manifests, event traces, and fail-closed runtime checks.**

```bash
kerna run "Research YC companies hiring AI engineers"
```

Every run is persistent, transparent, and bounded by strict policies.

---

## Why Kerna is different

Most agent frameworks help you build agents. Kerna helps you run agents securely.

It adds budgets, dynamic provider routing, plugin risk cards, structured traces, and fail-closed permission policies around any model or MCP tool.

---

## The Moat: Absolute Observability
Every single action, token, cost, tool call, failure, and permission check is recorded into Kerna's SQLite memory.

**Every task is reproducible.**

```bash
kerna trace last
```

You can easily inspect task lifecycle traces to debug policies and constraints:
```bash
kerna inspect last
```

## Features

- **Embedded Memory**: Built-in SQLite persistent task and episodic memory.
- **Fail-Closed Runtime Checks**: Strict trust boundaries. Agents cannot access your tools unless explicitly granted via policy or CLI overrides. Kerna operates in user-space (it is *not* a cryptographically isolated hypervisor), but heavily bounds agent action via application-level policy checks.
- **BYOK Provider Routing**: Bring Your Own Key architecture allows you to route tasks dynamically (e.g. `local-only`, `cheap`, `private`) preventing secret leakage.
- **Execution Guardrails**: Hard execution budgets (`max_tool_calls`, `max_runtime_seconds`, `max_llm_calls`) prevent runaway loops.
- **MCP Extensibility**: Native support for the Model Context Protocol, with strict fast-path tool filtering (`allow_tools`, `deny_tools`) and Risk Cards.

## Getting Started

1. Initialize Kerna and configure your baseline settings:
```bash
kerna init --quick
```

2. Add a model provider (BYOK):
```bash
kerna provider add openai --provider-type openai --api-key-env OPENAI_API_KEY
```

3. Check your system health:
```bash
kerna doctor
```

4. Run a supervised task:
```text
kerna run "Write a Python script to calculate fibonacci"
```

5. Simulate a dangerous action to see the Policy Engine block it:
```bash
kerna policy simulate "shell.exec" "{\"command\": \"rm -rf /\"}"
```

See the `docs/` folder for guides on the Security Model, Policy Engine, and BYOK Providers.
