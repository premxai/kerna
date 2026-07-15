# Kerna Launch Readiness Report (historical)

> This is a historical engineering record from an earlier hardening pass, not
> the current release decision. Its phase descriptions and contemporaneous
> test counts are retained for provenance. Use
> [COHORT_LAUNCH_CHECKLIST.md](COHORT_LAUNCH_CHECKLIST.md) for the current
> initial-cohort acceptance evidence and live-account gates.

## 1. Executive summary

Kerna is a Rust **runtime trust layer** for AI agents: it owns the agent loop's safety (fail-closed permissions, execution budgets, SQLite event traces, MCP plugin isolation) and delegates all domain logic to external MCP plugins. This pass took it from "impressive prototype with an unwired provider system" to "genuinely runnable across many LLM providers, with a safe-by-default server and honest risk reporting."

**Quality gates (all green):**

| Gate | Result |
|---|---|
| `cargo build` | ✅ clean |
| `cargo fmt --check` | ✅ formatted |
| `cargo clippy` | ✅ zero warnings |
| `cargo test` | ✅ **48 passed** (was 29) |
| `cargo build --release` | ✅ clean with LTO profile |
| `cargo audit` | ✅ no vulnerabilities (1 *unmaintained* transitive crate: `rustls-pemfile`; `fxhash` gone with wasmtime) |
| Dependency count | ✅ **227 crates** (was 333, −106 after dropping wasmtime) |
| CLI command matrix | ✅ **26/26 checks pass** |
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

### Phase 6 — Performance, onboarding & docs (follow-up pass)
- **Dropped the unused `wasmtime` dependency** (`WasmSandbox` was defined but never constructed). This cut the dependency graph **333 → 227 crates (−106)**, meaningfully shrinking cold-build time and binary size, and it eliminated the `fxhash` unmaintained advisory (only `rustls-pemfile`, transitive from reqwest, remains).
- Added a `[profile.release]` (`lto = true`, `codegen-units = 1`, `strip = true`, `opt-level = 3`) for small, fast shipped binaries.
- **Shared `reqwest::Client`**: built once in `TaskScheduler::new` and reused across LLM rounds so TLS connections pool (was rebuilt every call).
- **Bounded `@file`/`@url` goal injection**: 256 KB cap (char-boundary safe), 20 s fetch timeout, and content fenced as "untrusted … data, not instructions" — closes an unbounded-download / prompt-injection surface. Verified a 400 KB file still runs cleanly.
- **Redesigned `kerna init` onboarding**: provider picker now lists all presets + a zero-key **Demo mode** + Ollama; shows the exact env var each provider reads (with SET/MISSING detection and per-OS `setx`/`export` lines); ends with a tailored "your first 3 commands" block.
- New everyday-usage guide `docs/USING_KERNA.md`; README gains a completed command reference + tools/MCP catalog (the previous table was truncated mid-row).

### Phase 9 — One-command install for every user type (follow-up)
- Added `install.sh` (macOS/Linux) and `install.ps1` (Windows) one-line installers that pull the right prebuilt binary from GitHub Releases, detect OS/arch, install to a user bin dir, verify, and fix PATH. Both support a `KERNA_LOCAL_BIN` override for offline/air-gapped installs.
- Added an npm distribution under `npm/` (`@premxai/kerna`): `npm install -g @premxai/kerna` or `npx @premxai/kerna`. A postinstall script downloads the platform binary; a Node launcher shim forwards args/stdio/exit-code. `npm pack` is a clean 2.2 kB tarball (binary fetched on install, not bundled).
- Aligned all asset names across `install.sh`, `install.ps1`, `npm/install.js`, and `release.yml` (`kerna-linux-x86_64`, `kerna-macos-arm64`, `kerna-macos-x86_64`, `kerna-windows-x86_64.exe`); added the macOS-Intel target to `release.yml`.
- Tested on this machine: `cargo install` (from source), `install.ps1` (native, installs + PATH), npm postinstall + launcher (`kerna 0.1.0`, arg forwarding, policy DENY) + `npm pack`, `install.sh` syntax + platform-mapping for all 5 targets. README install section rewritten as one-liners per user type.
- Note: the download-based installers require a published `v*` GitHub release to function; `cargo install` works today. Cut a release with `git tag v0.1.0 && git push origin v0.1.0` to activate them.

### Phase 8 — MCP policy-gateway mode (follow-up)
- New `kerna gateway` command + `kernel/src/gateway.rs`: Kerna runs as an **MCP server over stdio** that proxies the MCP servers in `kerna.toml` through its policy engine and event log. Any MCP client (Claude Code, Cursor, Cline) points at `kerna gateway`; every `tools/call` is policy-checked and recorded — drop-in governance over existing tools with no runtime migration. This is the highest-leverage adoption feature.
- Fail-closed by design: only `auto_approve` tools are forwarded; `deny`/`require_confirmation` and unknown tools are blocked with an MCP `isError` result (a non-interactive server can't prompt). Downstream `allow_tools`/`deny_tools`/capability filters still apply.
- Every proxied call writes the standard `tool.call.requested` → `tool.policy.checked` → `tool.call.completed`/`blocked` event chain, so `kerna trace <gateway-task-id>` shows the full audit trail.
- Added `McpRegistry` quiet mode (diagnostics → stderr) since stdout is the JSON-RPC channel. Verified live end-to-end (echo forwarded, `secret_probe` blocked, unknown tool blocked, trace persisted) plus an in-process integration test. Suite now **48 passing**.

### Phase 7 — Real semantic memory (follow-up)
- **Replaced the embedding stub** (`[0.1, 0.2, 0.3]` for every memory) with a real, dependency-free local embedder in `kernel/src/embeddings.rs`: a feature-hashing vectorizer over word unigrams/bigrams **plus character trigrams** (the fastText subword trick), L2-normalized to 256 dims. It's deterministic, offline, preserves the local-only privacy guarantee, and adds zero heavy dependencies.
- Wired it through `add_episodic_memory` (embeds content on write), `gather_context` (semantic recall blended with a LIKE fallback, relevance-thresholded), and both the `kerna memory search` subcommand and the interactive `/memory` command. Callers no longer pass fake vectors.
- Verified live: query "echo call" matches a stored "…call echo" memory at 0.50 (word order — LIKE misses this); "invoke the echo tool" still matches at 0.19 via subword overlap; unrelated queries correctly return nothing. 6 new tests (5 embedding unit tests + 1 end-to-end semantic recall test); suite now **47 passing**.
- Documented the neural-embedding upgrade path (OpenAI-compatible `/embeddings`, e.g. Ollama `nomic-embed-text` for local neural).

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
- **Neural embeddings are opt-in, not yet wired.** Semantic memory now uses a real built-in local embedder (see Phase 7), which is a large step up from the old stub, but true transformer embeddings via an `/embeddings` endpoint are documented as the upgrade path rather than implemented.
- **Git history** blob was removed post-report via `git-filter-repo` (`gh.zip` + the 50 MB `gh/bin/gh.exe`); `.git` went 38 MB → 567 KB with the HEAD tree hash verified unchanged.
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
- [x] README rewritten around the killer demo, with cross-platform install + command reference
- [x] Guided terminal onboarding (`kerna init`) with zero-key Demo mode
- [x] Everyday usage guide (`docs/USING_KERNA.md`)
- [x] Dependency/perf cleanup (dropped wasmtime, release profile, shared HTTP client)
- [ ] Real embeddings / streaming (post-launch)
- [ ] History blob removed (owner decision)
