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

## Install

Pick the one line for your setup — each installs the `kerna` command:

**macOS / Linux** (one-line installer):
```bash
curl -fsSL https://raw.githubusercontent.com/premxai/kerna/main/install.sh | sh
```

**Windows** (PowerShell):
```powershell
irm https://raw.githubusercontent.com/premxai/kerna/main/install.ps1 | iex
```

**Node / npm** (any OS with Node ≥16):
```bash
npm install -g @premxai/kerna
# or run once, no install:  npx @premxai/kerna init
```

**Rust** (build from source, any OS with the [Rust toolchain](https://rustup.rs)):
```bash
cargo install --git https://github.com/premxai/kerna --bin kerna
```

**Prebuilt binaries** for manual download: the [Releases](https://github.com/premxai/kerna/releases) page has `kerna-linux-x86_64`, `kerna-macos-arm64`, `kerna-macos-x86_64`, and `kerna-windows-x86_64.exe`.

Then verify: `kerna --version` — and you're ready for `kerna init`.

> The installer and npm scripts download prebuilt binaries from GitHub Releases, so they work once a `v*` release is published; `cargo install` builds from source and works today.

## Getting started

```bash
# 1. Initialize — a guided setup picks your provider, policy, and budgets
kerna init

# 2. Add a provider key (or pick Ollama / Demo mode for zero-key)
kerna keys add openai

# 3. Check system health
kerna doctor

# 4. Run a supervised task
kerna run "Write a Python script to calculate fibonacci"

# 5. See exactly what the agent did
kerna trace last
```

## Command reference

| Command | What it does |
|---|---|
| `kerna init` | Guided onboarding: provider, policy, budgets (use `--quick`/`--ci` for non-interactive) |
| `kerna run "<goal>"` | Execute a goal through the agent loop (`--converse` to approve each tool, `--privacy local-only` to force local models) |
| `kerna trace <id\|last>` | Full event trace: every prompt, tool call, policy check, budget snapshot |
| `kerna inspect <id\|last>` | Task summary: duration, model, tools used, tokens, real cost |
| `kerna explain <id\|last>` | Step-by-step reasoning chain for a task |
| `kerna task list / show / replay / export` | Manage, replay, and export past tasks |
| `kerna keys add / list` | Guided API-key setup + live validation (keys never written to disk) |
| `kerna provider add / list / test / route` | Manage BYOK LLM providers and routing |
| `kerna mcp add / probe / inspect / risk / filter` | Manage MCP plugins and their risk cards |
| `kerna memory search / approve / reject` | Query and curate persistent memory |
| `kerna policy simulate "<tool>" '<args>'` | Dry-run a tool call against the policy engine |
| `kerna doctor` | System health: database, provider keys, plugins |
| `kerna serve [--bind <addr>] [--token <t>]` | OpenAI-compatible API server |
| `kerna gateway` | Run as an MCP server that governs + records your other MCP tools |
| `kerna daemon` / `kerna watch <url>` | Background scheduler + continuous watchers |

## Tools & MCP plugins

Kerna owns *no* domain logic — every capability is an MCP plugin spawned as an isolated child process. Ready-made, zero-dependency plugins ship in [`plugins/`](plugins/), grouped into one-command **packs**:

```bash
kerna pack list                    # productivity, dev
kerna pack install productivity    # search + notes + web, fail-closed
kerna secrets add search           # set the API key it needs (guided)
kerna mcp risk search              # read the risk card before granting
```

| Pack | Plugins (tools) |
|------|------|
| **productivity** | search (`web_search`), notes (`add_note`/`search_notes`/…), web (`fetch_url`/`read_page_text`) |
| **dev** | files (`read_file`/`write_file`/…), git (read-only), http (`http_get`/`http_post_json`) |

Every tool is fail-closed — a pack sets read tools to *require approval* and leaves the rest denied until you grant them. Connect any other MCP server too (`kerna mcp add fetch npx -y @modelcontextprotocol/server-fetch`), and Kerna governs it the same way.

Not a developer? Start with the [everyday guide](docs/EVERYDAY.md). Want recurring routines (a daily digest, morning news)? `kerna routine add daily-digest` and run `kerna daemon`.

Also included: `desktop` and `voice` reference plugins (need extra pip packages), and `mock`. Connect any other MCP server the same way — `kerna mcp add <name> <command> [args...]`:

```bash
kerna mcp add myserver python "path/to/mcp_server.py"
kerna mcp add fetch npx -y @modelcontextprotocol/server-fetch
```

See [plugins/README.md](plugins/README.md) for the full catalog and how to write your own.

New to Kerna? See the [everyday usage guide](docs/USING_KERNA.md).

## Policy-gateway mode: govern the MCP tools you already use

`kerna gateway` makes Kerna an **MCP server** that proxies your *other* MCP servers through its policy engine and event log. Point Claude Code, Cursor, or Cline at Kerna instead of directly at a tool server, and every tool call is policy-checked (fail-closed) and recorded — without changing your agent.

```jsonc
// e.g. in an MCP client config, replace a direct server entry with:
{ "command": "kerna", "args": ["gateway"] }
```

Kerna spawns the servers listed in your `kerna.toml`, re-exposes their tools, and for each call: checks your policy → forwards only `auto_approve` tools → blocks the rest with a clear error → writes a full trace. Audit any session with `kerna trace <id>`.

## Serving an OpenAI-compatible API

```bash
kerna serve                              # loopback only, no auth — safe default
kerna serve --bind 0.0.0.0 --token SECRET # network-exposed requires a bearer token
```

See `docs/` for the [Security Model](docs/SECURITY_MODEL.md), [Architecture](docs/architecture.md), [Policy Engine](docs/POLICY_ENGINE.md), [Budgets](docs/BUDGETS.md), and [BYOK Providers](docs/BYOK_PROVIDERS.md). Current launch status: [Launch Readiness Report](docs/LAUNCH_READINESS_REPORT.md).
