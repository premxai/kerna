<p align="center">
  <a href="https://kerna.run">
    <img src="website/assets/kerna-mark.svg" width="92" alt="Kerna mark" />
  </a>
</p>

<h1 align="center">Kerna</h1>

<p align="center">
  <strong>The runtime trust layer for AI agents.</strong><br />
  Govern MCP tools, bound execution, and keep a receipt for every run.
</p>

<p align="center">
  <a href="https://kerna.run">Website</a> ·
  <a href="https://github.com/premxai/kerna/releases">Releases</a> ·
  <a href="https://www.npmjs.com/package/@premxai/kerna">npm</a> ·
  <a href="docs/USING_KERNA.md">Usage guide</a> ·
  <a href="docs/BENCHMARKS.md">Benchmarks</a> ·
  <a href="CONTRIBUTING.md">Contributing</a>
</p>

<p align="center">
  <a href="https://github.com/premxai/kerna/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/premxai/kerna/ci.yml?branch=main&label=checks&style=flat-square" alt="CI status" /></a>
  <a href="https://github.com/premxai/kerna/releases"><img src="https://img.shields.io/github/v/release/premxai/kerna?display_name=tag&style=flat-square&color=D8753C" alt="Latest release" /></a>
  <a href="https://www.npmjs.com/package/@premxai/kerna"><img src="https://img.shields.io/npm/v/@premxai/kerna?style=flat-square&color=D8753C" alt="npm version" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT%20or%20Apache--2.0-F4F1EA?style=flat-square" alt="License" /></a>
</p>

## Give AI work. Keep the keys.

Kerna is a local-first Rust runtime for running AI agents safely. It sits between a model and the MCP tools that can affect files, services, and external systems. Before a tool runs, Kerna evaluates explicit permissions, policy, and budgets. Afterward, it records a structured receipt in local SQLite storage.

Kerna is not an agent framework and does not own domain logic. Your models and capabilities stay replaceable. Domain capabilities live in external MCP plugins, which Kerna runs as isolated child processes over stdio.

```text
goal → scheduler → policy + budgets → isolated MCP tool → local receipt
                  ↘ model provider ↗
```

## What Kerna gives an agent

| Capability | What it means |
| --- | --- |
| **Fail-closed permissions** | Tools receive no capability unless it is explicitly granted in `kerna.toml`. |
| **Human approvals** | Consequential actions can pause for a clear local decision. |
| **Hard budgets** | Limits for tool calls, LLM calls, runtime, cost, output, and memory writes stop runaway work. |
| **MCP isolation** | Plugins run as untrusted child processes with sandboxed working directories and watchdog cleanup. |
| **Receipts and traces** | Prompts, decisions, tool calls, payloads, cost, and failures are recorded in SQLite for inspection and replay. |
| **Provider choice** | Use supported BYOK providers, OpenAI-compatible endpoints, or local models without changing the trust boundary. |
| **Local-first operation** | No Kerna account is required for the core runtime, workspace, policies, or receipts. |

## Install

Choose one install path. All release-based installers use published Kerna artifacts.

### npm

```bash
npm install -g @premxai/kerna
```

### macOS or Linux

```bash
curl -fsSL https://raw.githubusercontent.com/premxai/kerna/main/install.sh | sh
```

### Windows PowerShell

```powershell
irm https://raw.githubusercontent.com/premxai/kerna/main/install.ps1 | iex
```

### Build from source

```bash
cargo install --git https://github.com/premxai/kerna --bin kerna
```

Verify the installation:

```bash
kerna --version
```

For manual binaries, desktop installers, plugin bundles, and SHA-256 checksum files, use the [latest release](https://github.com/premxai/kerna/releases/latest).

## First useful run

Start with the deterministic mock provider. It needs no API key and is ideal for learning the control loop.

```bash
# Create a fail-closed local workspace with the mock provider.
kerna init --ci --provider mock

# Add the curated local productivity pack.
kerna pack install productivity

# Confirm providers, plugins, and local storage are healthy.
kerna doctor

# Run a bounded task, then inspect the receipt.
kerna run "Prepare my morning brief"
kerna inspect last
kerna trace last
```

When you are ready to use a real model, run `kerna init` interactively and select your provider. The onboarding flow makes provider, policy, and budget choices visible before work begins.

## Daily-use workflows

Kerna is designed for useful, reviewable routines, not opaque autonomy.

| Workflow | Example | Safety posture |
| --- | --- | --- |
| Morning brief | Combine calendar, local notes, weather, and reading into one plan. | Read-first; external writes remain governed. |
| Research synthesis | Gather sources, summarize findings, and preserve the tool trace. | Inspectable source and tool history. |
| Meeting preparation | Pull permitted context and draft an agenda or follow-up. | Local workspace data stays under explicit grants. |
| Developer task | Read a repository, propose a patch, and require approval before changes. | File and shell capabilities remain explicitly scoped. |
| Existing MCP clients | Place Kerna in front of configured MCP servers using gateway mode. | Policy checks and receipts apply without replacing the client. |

Explore the [everyday guide](docs/EVERYDAY.md) and [full usage guide](docs/USING_KERNA.md) for practical walkthroughs.

## Work with MCP tools

Kerna discovers MCP tools from your workspace configuration, classifies their risk, and governs every invocation. Installing a plugin does not silently grant it broad access.

```bash
# Browse available packs and install a curated set of local productivity tools.
kerna pack list
kerna pack install productivity

# See configured connectors and review a connector's risk card.
kerna mcp list
kerna mcp risk <connector-name>
```

The included packs cover local notes, calendar, weather, web reading, optional search, developer tools, and optional Google Workspace integration. See the [plugin catalog](plugins/README.md) and [plugin manifest specification](docs/PLUGIN_MANIFEST.md).

## Inspect and explain work

Every run becomes a local, structured record. Use the CLI to review what happened instead of treating an agent as a black box.

```bash
kerna inspect last      # concise task summary
kerna trace last        # events, tool calls, policy checks, and cost data
kerna explain last      # human-readable explanation of the completed run
kerna task list         # recent task records
```

## Architecture and security model

Kerna deliberately keeps a narrow core:

- **Scheduler:** manages the task loop, memory injection, model calls, tools, and lifecycle state.
- **Policy engine:** denies ungranted capabilities by default and can require approval for sensitive actions.
- **Budget engine:** aborts runs that exceed declared limits.
- **MCP registry:** spawns and supervises external plugins over stdio.
- **SQLite event store:** retains structured observability data and memory locally.

Read the [architecture](docs/architecture.md), [security model](docs/SECURITY_MODEL.md), [policy engine](docs/POLICY_ENGINE.md), [budget guide](docs/BUDGETS.md), [event schema](docs/EVENT_SCHEMA.md), and [provider guide](docs/BYOK_PROVIDERS.md) before changing one of these boundaries.

> Kerna reduces risk through explicit policy, isolation, budgets, and visibility. It does not make an unsafe plugin safe by itself. Review connector risk cards, grant the minimum capability required, and keep consequential actions behind approval.

## Run locally from source

The Rust workspace lives in `kernel/`.

```bash
cd kernel
cargo build
cargo test
cargo fmt -- --check
cargo clippy -- -D warnings
```

Run the end-to-end mock pipeline from the repository root:

```bash
scripts/smoke_test.ps1    # Windows PowerShell
# or
./scripts/smoke_test.sh   # macOS / Linux
```

The optional desktop interface is in `ui/`:

```bash
cd ui
npm install
npm run dev
```

## Project status

Kerna `v0.2.3` is the current public release. The core runtime, verified release artifacts, npm launcher, curated plugin bundle, website, and launch workflows are available now. See [releases](https://github.com/premxai/kerna/releases), [open issues](https://github.com/premxai/kerna/issues), the [benchmark methodology](docs/BENCHMARKS.md), and the [launch checklist](docs/COHORT_LAUNCH_CHECKLIST.md) for current work.

## Contributing

Contributions are welcome. Please read [CONTRIBUTING.md](CONTRIBUTING.md), keep pull requests focused, and include tests. Changes that add domain-specific behavior to the core will generally be better as MCP plugins, preserving Kerna's architecture boundary.

For security vulnerabilities, do not open a public issue. Follow [SECURITY.md](SECURITY.md).

## License

Licensed under either of:

- MIT License
- Apache License, Version 2.0

at your option. See [LICENSE](LICENSE).
