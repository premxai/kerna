# Using Kerna — Everyday Guide

This guide is for people who want to *use* Kerna to get work done — connect tools, run agents safely, and keep a record of everything. If you want the internals, read [architecture.md](architecture.md).

## The mental model

Kerna is a **trust layer**, not an agent. Think of it as the seatbelt + black-box recorder that sits between a language model and your machine:

```
   You  ──goal──▶  Kerna  ──▶  LLM (picks a tool)
                     │
                     ├─ permission check (fail-closed)
                     ├─ budget check (hard limits)
                     ├─ run tool in isolated MCP process
                     └─ record everything to SQLite
```

Kerna itself does nothing domain-specific. Every real capability — files, desktop, voice, your APIs — is an **MCP plugin** that Kerna spawns as an isolated child process and governs.

## Your first five minutes

```bash
kerna init                 # guided setup (pick Demo mode to try with zero keys)
kerna run "Please call echo"   # watch a full agent loop
kerna trace last           # see every step it took
kerna doctor               # confirm keys + plugins are healthy
```

## Connecting a model

Kerna is BYOK (bring your own key). Ten providers work out of the box:

`openai` · `anthropic` · `openrouter` · `ollama` · `groq` · `together` · `deepseek` · `mistral` · `xai` · `venice`

```bash
kerna keys add openai      # tells you which env var to set, then validates it live
kerna provider list        # what's configured
```

Keys are read from environment variables and **never written to disk**. To go fully local (no key, no data leaving your machine), use Ollama:

```bash
ollama serve && ollama pull qwen2.5-coder
kerna provider add ollama
kerna run "Summarize notes.md" --privacy local-only   # refuses to run on a non-local endpoint
```

Any other OpenAI-compatible endpoint works too:

```bash
kerna provider add myhost --base-url https://my-endpoint/v1 --api-key-env MY_KEY
```

## Connecting tools (MCP plugins)

A tool is anything exposed by an MCP server over stdio. Kerna ships three reference plugins in [`plugins/`](../plugins/), and you can add any other:

```bash
kerna mcp add filesystem --command python --args "plugins/desktop_mcp/mcp_server.py"
kerna mcp risk filesystem      # read the risk card BEFORE granting anything
kerna mcp list                 # what's connected and what tools they expose
```

Nothing a plugin exposes can actually run until you grant it in `kerna.toml`. That's the whole point.

## Granting permissions (the safety dial)

By default every tool is **denied**. You opt in per tool:

```toml
# kerna.toml
[[permissions]]
tool = "write_file"
action = "require_confirmation"   # pauses and asks you mid-run

[[permissions]]
tool = "read_file"
action = "auto_approve"           # runs without asking

[[permissions]]
tool = "*"
action = "deny"                   # everything else: blocked
```

Three actions: `auto_approve`, `require_confirmation`, `deny`. Dangerous built-ins (delete, format, desktop control, send-email) escalate to confirmation even if auto-approved. Test any call without running it:

```bash
kerna policy simulate run_command '{"command":"rm","args":["-rf","/"]}'
# → DENY: operates destructively outside the workspace boundary
```

## Keeping agents on a budget

Every task runs under hard limits so a loop can't run forever or run up a bill:

| Limit | Stops |
|-------|-------|
| `max_tool_calls` | tool-call loops that never finish |
| `max_llm_calls` | runaway model calls |
| `max_runtime_seconds` | tasks that hang |
| `max_cost_usd` | surprise bills |
| `max_output_bytes` | context blowups from huge outputs |
| `max_memory_writes` | memory poisoning |

Pick a preset during `kerna init` (`conservative` / `balanced`) or set them in `kerna.toml`.

## Everyday workflows

**Research with a source:**
```bash
kerna run "Summarize @https://example.com/post and list 3 takeaways"
```
(Fetched content is size-capped and clearly fenced as untrusted data, not instructions.)

**Work on a local file:**
```bash
kerna run "Refactor @src/util.py for readability, explain your changes"
```

**Approve each step (max control):**
```bash
kerna run "Clean up my downloads folder" --converse
```

**Audit what happened:**
```bash
kerna inspect last     # duration, model, tools, real token cost
kerna explain last     # the reasoning chain
kerna trace last       # the full forensic event log
kerna task replay <id> # re-run an old task
```

## How memory recall works

Every task can leave a memory (staged for your approval — `kerna memory staged` / `approve`). When a new task starts, Kerna injects the most *relevant* past memories into the prompt — not just keyword matches.

Relevance uses a built-in **local embedder**: each memory is turned into a vector and ranked by cosine similarity to your goal. It's fully offline (no embedding API call), so it works under `--privacy local-only` and never sends your memories anywhere. Because it uses subword features, it matches across word forms — a goal about "deleting a file" recalls a memory about "file deletion" even with no shared exact word.

```bash
kerna memory search "how do I remove a file"   # ranked by similarity, not substring
```

Want true neural embeddings? Point Kerna at any OpenAI-compatible `/embeddings` endpoint — e.g. Ollama's `nomic-embed-text` for local neural embeddings, or OpenAI's `text-embedding-3-*`. (Upgrade path; the local embedder is the zero-config default.)

## Governing tools you already use (gateway mode)

You don't have to run agents *through* Kerna to benefit from it. `kerna gateway` puts Kerna in front of your existing MCP tools as a proxy: your MCP client (Claude Code, Cursor, Cline) talks to Kerna, and Kerna talks to the real tool servers — applying policy and recording everything in between.

```jsonc
// point your MCP client at Kerna instead of the tool server directly:
{ "command": "kerna", "args": ["gateway"] }
```

Kerna reads the downstream servers from your `kerna.toml`, re-exposes their tools, and for each call:
- **checks your policy** — only `auto_approve` tools pass; `deny` / `require_confirmation` / unknown tools are blocked with a clear error (a background proxy can't stop to ask you, so it fails closed);
- **records a trace** — `kerna trace <id>` shows every proxied call, its policy decision, and result.

To allow a downstream tool through, grant it in `kerna.toml`:
```toml
[[permissions]]
tool = "read_file"
action = "auto_approve"
```

## Where things live

| File / dir | What it is |
|------------|-----------|
| `kerna.toml` | Your config: providers, permissions, budgets, MCP servers |
| `kerna.db` | SQLite: every task, event, and memory (WAL mode) |
| `./sandbox` | Isolated working directory tools run inside |

## Troubleshooting

- **"No API key for provider"** → run `kerna keys add <provider>` and follow the env-var instructions.
- **A tool "Denied by policy"** → it isn't granted in `kerna.toml`; add a permission rule or run with `--converse`.
- **Plugin hangs** → Kerna kills and reaps it automatically; the trace records the failure. Check `kerna mcp probe <name>`.
- **Anything unexpected** → `kerna doctor` first, then `kerna trace last` to see exactly what happened.
