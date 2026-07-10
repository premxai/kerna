# Kerna Launch Readiness Report

_Generated during the pre-launch hardening pass. Reflects the state of the `kernel/` crate after Phases 0–4._

## 1. Executive summary

Kerna is a Rust **runtime trust layer** for AI agents: it owns the agent loop's safety (fail-closed permissions, execution budgets, SQLite event traces, MCP plugin isolation) and delegates all domain logic to external MCP plugins. This pass took it from "impressive prototype with an unwired provider system" to "genuinely runnable across many LLM providers, with a safe-by-default server and honest risk reporting."

**Quality gates (all green):**

| Gate | Result |
|---|---|
| `cargo build` | ✅ clean |
| `cargo fmt --check` | ✅ formatted |
| `cargo clippy` | ✅ zero warnings |
| `cargo test` | ✅ **41 passed** (was 29) |
| `cargo audit` | ✅ no vulnerabilities (2 low-priority *unmaintained* transitive crates: `fxhash`, `rustls-pemfile`) |
| CLI command matrix | ✅ **24/24 checks pass** |
| Server auth (live HTTP) | ✅ 401 without token, 200 + real output with token |
| Smoke test (`scripts/smoke_test.sh`) | ✅ pipeline trace verified |

## 2. What changed this pass

### Phase 0 — Repo hygiene
- Removed `gh.zip` (13 MB), `gh/` (vendored GitHub CLI), `check.py` (corrupt UTF-16), `fetch_actions.py`. Added `*.zip` and `/gh/` to `.gitignore`.
- **Action for the owner:** git history still contains the 13 MB blob. Before the repo gains traction, squash history or run `git filter-repo` to drop it. (Not done here — history rewrites need explicit owner approval.)

### Phase 1 — Provider engine (the core rewrite)
- New `kernel/src/providers.rs`: single source of truth resolving a provider name → `(wire protocol, base_url, api_key, model)`. Built-in presets for **OpenAI, Anthropic, OpenRouter, Ollama, Groq, Together, DeepSeek, Mistral, xAI, Venice**; any OpenAI-compatible host works via `kerna provider add <name> --base-url <url>`.
- `execute_llm_call`'s hardcoded `openai|venice|anthropic` branch replaced by `call_openai_compat` + `call_anthropic`, dispatched by resolved protocol. Ollama/OpenRouter now actually work.
- **Anthropic multi-turn tool loop fixed**: assistant `tool_calls` → `tool_use` blocks; `tool`-role results → coalesced `tool_result` blocks in the following user turn (preserves required alternation). Previously tool results were dropped, so agentic loops with Claude could never complete. Covered by 3 new unit tests.
- **Real token/cost accounting**: per-call `usage` tokens now thread into the budget tracker and task observability, with a static pricing table (`estimate_cost_usd`). The old `round * 450` mock is gone.
- **`--privacy local-only` is now a hard guarantee**: if the resolved endpoint is not loopback, the run is refused.

### Phase 2 — Key management UX
- New `kerna keys add <provider>` (guided env-var setup, per-shell copy-paste lines, **live key validation** via a cheap read-only call) and `kerna keys list` (SET/MISSING per provider). Keys are never written to disk — the existing serde-skip invariant and its test are preserved.
- `kerna doctor` now reports the active provider + per-provider key status instead of a single "LLM Key: MISSING".

### Phase 3 — Safety hardening
- **API server**: defaults to `127.0.0.1`; binding a non-loopback address requires `--token`. Bearer auth enforced when a token is set. All `.unwrap()`s replaced with proper JSON error responses. **Returns the real final assistant message** (persisted via a new `result_text` column) instead of a canned string.
- **Lazy MCP registry**: read-only/observability commands (`trace`, `inspect`, `task`, `memory`, `config`, `policy`, `provider`, `keys`, `doctor`) no longer spawn plugins or print the registration banner.
- **Risk classifier is now fail-closed**: a tool is auto-allowed only if its name clearly denotes a read-only op; dangerous, secret/network-touching, mutating, or **unrecognized** tools all require review. (`secret_probe`/`network_probe` are no longer labeled "Safe".)
- **MCP client robustness**: JSON-RPC responses are matched by request `id`; notification and noise lines are skipped within a bounded loop.
- **`classify_command` hardening**: shell/interpreter inline-code wrappers (`bash -c`, `powershell -Command`, `cmd /C`, `python -c`, …) classified DangerousGlobal; install-block list extended (`pip3`, `python -m pip`, `apt`, `brew`, `winget`, `choco`, `yarn`, `pnpm`, `gem`, `go install`).
- Documented the prompt-injection detector as heuristic-only in `SECURITY_MODEL.md` (the real control is the fail-closed permission + budget layer).

### Earlier in the session (pre-plan)
- Fixed a Windows/UNC absolute-path bypass in the workspace-boundary check (`rm C:\Windows` was allowed; now denied).
- Fixed task duration reporting a Unix epoch instead of elapsed time.

## 3. Command matrix results

All exercised against the mock provider + MockMCP (no real keys):

| Command | Result |
|---|---|
| `--help`, `--version` | ✅ |
| `init --ci` | ✅ |
| `doctor` (DB, active provider, per-provider keys) | ✅ |
| `run` (mock agent loop) | ✅ completes, real result persisted |
| `trace` / `inspect` / `explain` | ✅ pipeline events, correct duration |
| `task list` | ✅ no plugin banner (lazy MCP) |
| `policy simulate` — `rm -rf /`, `rm C:\Windows`, `shell.exec` | ✅ all DENY |
| `provider add` (preset) / `provider list` | ✅ presets pre-fill |
| `keys list` / `keys add <local>` / `keys add <remote>` | ✅ guidance + validation |
| `mcp risk` / `mcp list` | ✅ fail-closed classification |
| `memory search`, `config path` | ✅ |
| `serve --bind 0.0.0.0` (no token) | ✅ refused |
| `serve` loopback + token → HTTP | ✅ 401 w/o token, 200 + real output w/ token |

## 4. Security probe results (post-fix)

| Probe | Expected | Result |
|---|---|---|
| `rm -rf /` | DENY | ✅ |
| `rm C:\Windows\System32` (Windows abs) | DENY | ✅ |
| `\\server\share` (UNC), `..` traversal | DENY | ✅ |
| `bash -c "rm -rf /"` (shell wrapper) | DangerousGlobal | ✅ |
| Denied tool in agent loop | fail-closed | ✅ "Denied by policy" |
| Budget exceeded (tool/llm/memory caps) | abort | ✅ (integration tests) |
| Hanging plugin | SIGKILL + continue | ✅ (`test_mockmcp_hang_times_out_cleanly`) |
| Unauthenticated non-loopback serve | refuse | ✅ |
| Unknown MCP tool in risk card | not auto-allowed | ✅ requires review |

## 5. Known gaps / not yet done

- **Streaming responses** (`stream: true`) are accepted but ignored by the server; it returns a single completion.
- **Embeddings are still a stub** (`[0.1, 0.2, 0.3]`) — semantic memory retrieval is not yet real vector search.
- **Git history** still carries the removed 13 MB blob (see Phase 0).
- **Live provider calls** were validated against OpenAI's real 401 path; a full happy-path call against each provider requires user-supplied keys / a running Ollama and is marked *pending user keys*.
- `provider test` still prints a simulated success — superseded in practice by `keys add`'s real validation, but the old subcommand text remains.

## 6. Launch checklist

- [x] Multi-provider LLM support (10 built-in + any OpenAI-compatible)
- [x] User can add keys (guided, validated, never on disk)
- [x] Tools / tool usage / MCP plugins working with fail-closed policy
- [x] Safe-by-default server (loopback + auth)
- [x] Full command suite tested end-to-end
- [x] CI runs fmt + clippy + audit + tests + smoke on 3 OSes
- [x] Release workflow producing tagged binaries (win/mac/linux)
- [ ] README rewritten around the killer demo (Phase 5)
- [ ] Real embeddings / streaming (post-launch)
- [ ] History blob removed (owner decision)
