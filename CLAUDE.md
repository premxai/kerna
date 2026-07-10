# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What Kerna Is

Kerna is a Rust runtime ("trust layer") for autonomous AI agents. It is *not* an agent framework — it owns orchestration and safety (scheduler, budgets, fail-closed permissions, SQLite memory/observability, MCP process isolation) and deliberately owns **no domain logic**. All domain capabilities live in external MCP plugins spawned as untrusted child processes over stdio. If a change makes the kernel depend on what a plugin does, it violates the architecture boundary (see `docs/architecture.md`).

## Commands

All Rust work happens in `kernel/` (the only workspace member):

```bash
cd kernel
cargo build                 # build the `kerna` binary
cargo test                  # run all tests
cargo test <name>           # run a single test (e.g. cargo test permissions)
cargo clippy -- -D warnings # CI enforces zero clippy warnings
cargo fmt -- --check        # CI enforces formatting
```

CI (`.github/workflows/ci.yml`) runs fmt, clippy `-D warnings`, `cargo audit`, build, and tests on Linux/Windows/macOS — run fmt and clippy before committing.

End-to-end smoke test (from repo root; writes a `kerna.toml` and drives the mock LLM + MockMCP pipeline):

```bash
./scripts/smoke_test.sh     # or scripts/smoke_test.ps1 on Windows
```

Desktop UI (Tauri 2 + React 19 + Vite, in `ui/`):

```bash
cd ui
npm run dev        # vite dev server
npm run build      # tsc && vite build
npm run tauri dev  # run desktop app
```

## Architecture

### Task lifecycle (the core loop)

`kerna run <goal>` drives this pipeline, mostly implemented in `kernel/src/scheduler.rs` (the largest module):

1. **Memory injection** — `memory.rs` (SQLite `kerna.db`, WAL mode) retrieves relevant past task summaries into the system prompt.
2. **Tool discovery** — MCP servers listed in `kerna.toml` are spawned (`mcp.rs`, `mcp_registry.rs`) and queried for tools.
3. **LLM loop** — goal + memory + tool schema sent to the provider (`gateways.rs`, BYOK routing via `providers`/`model_routes`/`privacy_routes` in config).
4. **Permission validation (fail-closed)** — before any tool call, `permissions.rs` checks capabilities granted in `kerna.toml`; ungranted → denied. Dangerous capabilities can require human approval mid-loop.
5. **Budget enforcement** — `budget.rs` hard limits (`max_tool_calls`, `max_runtime_seconds`, `max_llm_calls`) abort runaway loops.
6. **Tool invocation** — request sent to the isolated MCP child process (sandboxed working dir via `sandbox.rs`; hung plugins are SIGKILLed and reaped by `watchdog.rs`).
7. **Everything recorded** — every prompt, tool call, JSON payload, cost, and permission check is written to SQLite as structured events (`events.rs`, schema in `docs/EVENT_SCHEMA.md`), making tasks replayable via `kerna trace` / `kerna inspect` / `kerna explain`.

### Key modules in `kernel/src/`

- `main.rs` — clap CLI defining all subcommands (`init`, `run`, `daemon`, `serve`, `trace`, `inspect`, `task`, `policy`, `provider`, `doctor`, `mockmcp`, ...).
- `config.rs` — `Config` struct mirroring `kerna.toml` (providers, MCP servers, permissions, schedules, budget presets, `runtime_mode`).
- `trust_layer_validation.rs` — integration test suite exercising the full scheduler + memory + budget pipeline; add end-to-end trust-boundary tests here.
- `mockmcp.rs` — built-in deterministic MCP server (`kerna mockmcp`) used by tests and the smoke test; use it to test the pipeline without real plugins or API keys (set `llm_provider = "mock"`).
- `mcp_governance.rs`, `plugin_manifest.rs`, `tool_packs.rs` — plugin manifest validation, risk cards, and `allow_tools`/`deny_tools` filtering.
- `server.rs` — OpenAI-compatible API server (`kerna serve`); `cron.rs` + `watchdog.rs` back `kerna daemon`.

### Other directories

- `plugins/` — Python reference MCP plugins (`mock_mcp`, `desktop_mcp`, `voice_mcp`), each a single `mcp_server.py` speaking MCP over stdio.
- `docs/` — the source of truth for subsystem design: `architecture.md`, `SECURITY_MODEL.md`, `POLICY_ENGINE.md`, `BUDGETS.md`, `EVENT_SCHEMA.md`, `BYOK_PROVIDERS.md`, `PLUGIN_MANIFEST.md`. Consult these before changing the corresponding subsystem.
- `ui/src-tauri/` — separate Cargo project for the Tauri shell (not part of the `kernel` workspace).

## Conventions

- **Fail-closed is the invariant**: any new capability, tool path, or permission check must deny by default. Never add a bypass that grants access absent an explicit `kerna.toml` grant.
- Strict test coverage is expected for `permissions.rs` and `memory.rs` (per CONTRIBUTING.md); unit tests live in `mod tests` blocks within each module.
- New agent-facing functionality should be an MCP plugin, not a kernel feature — the maintainers reject domain logic in the core runtime.
