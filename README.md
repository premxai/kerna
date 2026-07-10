# Kerna

**The Runtime Trust Layer for AI Agents**

AI agents can think, but they run with no seatbelt: full system access, no memory of what they did, no way to prove what happened. Kerna is the runtime that wraps *any* model or MCP tool stack in a hard safety boundary — budgets, fail-closed permissions, full event traces, and process isolation — so you can run autonomous agents without fearing destructive commands or losing the record.

Kerna doesn't compete with your agent framework. It runs underneath it.

```bash
kerna run "Research YC companies hiring AI engineers"
```

Every run is persistent, bounded by strict policy, and reproducible from a durable trace.

---

## The 60-second demo

**1. Try it with zero API keys** (mock model + built-in MockMCP):

```bash
kerna init --quick
kerna run "Please call echo"
```

**2. Watch the trust layer block a destructive command:**

```bash
kerna policy simulate run_command '{"command":"rm","args":["-rf","/"]}'
# → Final Decision: DENY  (operates destructively outside the workspace boundary)
```

Windows paths, UNC paths, `..` traversal, and shell wrappers like `bash -c "rm -rf /"` are all caught the same way.

**3. Inspect exactly what happened:**

```bash
kerna trace last      # every tool call, policy check, budget snapshot, token/cost
kerna inspect last    # duration, model, tools used, real cost
```

## Bring your own model

Ten providers work out of the box, plus any OpenAI-compatible endpoint:

```bash
kerna keys add openai            # guided setup; validates the key live; never writes it to disk
kerna provider add ollama        # fully local — no key, no data leaves your machine
kerna run "Summarize README.md" --privacy local-only   # refuses to run if the endpoint isn't local
```

Built-in presets: `openai`, `anthropic`, `openrouter`, `ollama`, `groq`, `together`, `deepseek`, `mistral`, `xai`, `venice`. Add any other with `kerna provider add <name> --base-url <url>`.

## Why Kerna is different

Most agent frameworks help you *build* agents. Kerna helps you *run them safely*:

- **Fail-closed permissions** — by default an agent has zero privileges. It can't touch the filesystem, network, or shell unless you grant the capability in `kerna.toml`. Dangerous tools can require human approval mid-loop.
- **Execution budgets** — hard limits (`max_tool_calls`, `max_llm_calls`, `max_runtime_seconds`, `max_cost_usd`, `max_output_bytes`, `max_memory_writes`) stop runaway loops and runaway bills.
- **Absolute observability** — every action, token, cost, tool call, failure, and permission check is recorded to embedded SQLite. Every task is reproducible.
- **Process isolation** — MCP plugins run as untrusted child processes in a sandboxed working dir; hung plugins are killed and reaped, and the agent chooses another path.
- **BYOK privacy routing** — route sensitive tasks to local models so secrets never leave the machine.

## Getting started

```bash
# 1. Build (Rust toolchain required)
cd kernel && cargo build --release

# 2. Initialize
kerna init --quick

# 3. Add a provider key (or use Ollama for zero-key local)
kerna keys add openai

# 4. Check system health
kerna doctor

# 5. Run a supervised task
kerna run "Write a Python script to calculate fibonacci"
```

## Serving an OpenAI-compatible API

```bash
kerna serve                              # loopback only, no auth — safe default
kerna serve --bind 0.0.0.0 --token SECRET # network-exposed requires a bearer token
```

See `docs/` for the [Security Model](docs/SECURITY_MODEL.md), [Architecture](docs/architecture.md), [Policy Engine](docs/POLICY_ENGINE.md), [Budgets](docs/BUDGETS.md), and [BYOK Providers](docs/BYOK_PROVIDERS.md). Current launch status: [Launch Readiness Report](docs/LAUNCH_READINESS_REPORT.md).
