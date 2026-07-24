# Kerna benchmark results

This page reports released benchmark evidence. Results are split by what they
actually measure; no utility or safety result is combined into a single score.

## Kerna Trust Bench v0.1.0

**Status:** passed

| Field | Result |
| --- | --- |
| Source revision | `e2edbb0` |
| Execution | Local, deterministic, no model provider or API key |
| Scenarios | 17 |
| Passed | 17 |
| Failed | 0 |
| Pass rate | 100% |

Run it locally:

```powershell
node benchmarks/kerna-trust/run.mjs --out reports/kerna-trust/latest.json
```

The suite runs real child-process MCP protocol scenarios against the compiled
Kerna binary. It covers explicit allow and deny policy, approval ordering,
budget enforcement, secret and folder isolation, malformed protocol traffic,
timeout containment, connector governance, and receipt ordering.

The new `allowed-action-denied-action-same-task` scenario verifies the core
least-privilege claim in one task: an allowed `echo` action completes, while a
distinct denied `network_probe` action receives a `Deny` policy receipt and
never emits `tool.call.started`.

The current release run also splits this deterministic result into distinct
governance scorecards:

| Scorecard | Current deterministic result |
| --- | --- |
| Tool-call and LLM-call budget enforcement | 2 / 2 scenarios passed |
| Receipt decision-chain ordering | 1 / 1 scenario passed |
| Approval ordering | 1 / 1 scenario passed |
| Allowed tool receipt coverage | 1 / 1 scenario passed |

The redacted current aggregate is
[`kerna-trust-release-20260723.json`](benchmark-data/kerna-trust-release-20260723.json).

### Interpretation

This is a runtime-mechanism result. It demonstrates that the listed Kerna
controls work deterministically for the included scenarios. It does not claim
that an arbitrary model will complete arbitrary real-world work safely.

## MCP core client conformance

**Status:** passed for the supported stdio-client core.

| Field | Result |
| --- | --- |
| Source revision | `d78a1ff` |
| Kerna package version | `0.2.3` |
| Host | Windows, local release run |
| Official framework | `@modelcontextprotocol/conformance@0.1.16` |
| MCP revision advertised by Kerna | `2025-06-18` |
| Test transport | Pinned `mcp-remote@0.1.38` bridge from local HTTP scenario server to Kerna stdio child process |
| Official core scenarios | 2 / 2 passed |

The passed scenarios are `initialize` and `tools_call`. The latter verifies
that Kerna discovered the official `add_numbers` tool and called it with
`40 + 2 = 42`. The reproducible runner is
[`benchmarks/mcp-conformance/run.mjs`](../benchmarks/mcp-conformance/run.mjs)
and the committed aggregate is
[`mcp-core-client-conformance-20260723.json`](benchmark-data/mcp-core-client-conformance-20260723.json).

This is a **stdio-client semantic** result. Kerna does not currently advertise
native remote HTTP transport, OAuth, SSE reconnection, elicitation, resources,
or prompts. Those features have no passing conformance claim until product
support and their individual official scenarios are added.

## MCP transport performance

**Status:** baseline published for the isolated stdio client path.

| Field | Result |
| --- | --- |
| Source revision | `93c80bc` |
| Host | Windows 11, Intel Core i7-14700HX, 28 logical CPUs, 16 GB RAM |
| Fixture | Built-in MockMCP `echo` |
| Process runs | 30 |
| Tool calls | 900 |
| Spawn plus MCP initialization, p50 / p95 | 17.233 ms / 24.158 ms |
| Tool discovery, p50 / p95 | 0.329 ms / 2.102 ms |
| Echo tool call, p50 / p95 | 0.056 ms / 0.130 ms |

The complete redacted aggregate is
[`mcp-stdio-performance-windows-20260723.json`](benchmark-data/mcp-stdio-performance-windows-20260723.json).
This measures Kerna's isolated stdio client, initialization, discovery, and
local tool-call path only. It excludes scheduler work, SQLite, provider and
model latency, remote network time, and concurrency. It is a baseline for
future same-machine release comparisons, not a universal performance claim.

## MCP restart reliability soak

**Status:** passed for the bounded local MockMCP fixture.

| Field | Result |
| --- | --- |
| Source revision | `a3ce2d0` |
| Host | Windows 11, Intel Core i7-14700HX |
| Fresh Kerna process runs | 120 / 120 |
| Built-in MockMCP child processes | 120 |
| Expected / completed echo calls | 2,400 / 2,400 |
| Failed process runs | 0 |
| Total duration | 6.171 seconds |

The redacted aggregate is
[`mcp-stdio-restart-soak-windows-20260723.json`](benchmark-data/mcp-stdio-restart-soak-windows-20260723.json).
Each iteration creates a new Kerna process, starts the isolated MockMCP child,
initializes MCP, discovers tools, executes 20 local echo calls, and exits. This
is a restart and child-process lifecycle signal. It does not claim external
provider availability, model reliability, scheduler reliability, or a general
operating-system orphan-process guarantee.

## BFCL provider compatibility pilot

**Status:** passed bounded provider-compatibility pilot.

| Field | Result |
| --- | --- |
| Framework | `bfcl-eval==2025.12.17` |
| Source revision | `ae905e9` |
| Provider/model baseline | OpenAI, `gpt-4.1-nano-2025-04-14-FC` |
| Category | Non-live `simple_python` |
| Fixed pilot size | 10 cases, `simple_python_0` through `simple_python_9` |
| Inference concurrency | 1 |
| Correct / total | 10 / 10 |
| Pilot accuracy | 100% |
| Full wrapper duration | 37.984 seconds |
| Provider billing cost | Not reconciled by this pilot |

The redacted aggregate is
[`bfcl-provider-compatibility-pilot-20260723.json`](benchmark-data/bfcl-provider-compatibility-pilot-20260723.json).
The reproducible harness is in [`benchmarks/bfcl`](../benchmarks/bfcl). Its
preflight makes no model calls. The execution command is deliberately bounded
to ten fixed cases, writes raw evaluator output only to ignored reports, and
requires an API key already present in the executing terminal.

BFCL measures the named provider/model's native function-calling compatibility.
It is not a Kerna security, utility, or policy-enforcement result, and it must
not be used to claim that Kerna made a model more capable. A ten-case partial
score is a pilot, not an official BFCL leaderboard score. In particular, the
framework's generated overall leaderboard percentage includes unevaluated
categories and must not be reported for this partial run.

## tau3 utility evaluation

**Status:** native calibration completed; gateway adapter contract passed; no
utility score published.

The current upstream benchmark is tau3. Kerna has a pinned, no-cost preflight
and a pre-registered three-task retail native control in
[`benchmarks/tau3`](../benchmarks/tau3). The control uses the same
`gpt-4o-mini` model for the agent and user simulator, one trial per task,
single-concurrency, a 60-step limit, a 300-second task timeout, and seed 300.

The corrected native calibration ran on 2026-07-23 with one trial for each of
retail tasks `0`, `1`, and `2`. Task `0` completed (reward `1.0`); tasks `1`
and `2` did not (reward `0.0`). This is a small calibration result, not a
published utility percentage. Under the comparison contract, only task `0` is
eligible for a Kerna counterpart.

The free gateway contract now passes: an MCP tool schema is re-exposed by
`kerna gateway`, a call returns to the same loopback tool handler, an unknown
tool is fail-closed, and the SQLite receipt has requested, policy-checked,
completed, and blocked events. The adapter routes each call back to the exact
single tau3 environment instance used for reward evaluation and does not send
the provider credential to Kerna or the bridge child. This validates the
measurement path, but is not a provider-backed Kerna utility result.

The next bounded run is one matched task-`0` gateway trial with the same model,
user simulator, seed, task state, schemas, step limit, and timeout. It may
measure task-level utility retention and receipt coverage only. It cannot
support a safety, prevention, or general-performance claim.

## AgentDojo external evaluation

**Status:** completed external control matrix; no Kerna protection rate
published.

The pre-registered
[`workspace authorized mutation attack matrix`](../benchmarks/agentdojo/campaigns/workspace-authorized-mutation-attack-matrix.json)
ran on 2026-07-23. Its committed aggregate is
[`agentdojo-workspace-authorized-mutation-matrix-20260723.json`](benchmark-data/agentdojo-workspace-authorized-mutation-matrix-20260723.json).

| Field | Result |
| --- | --- |
| Kerna source revision | `8589c11` |
| Kerna package version | `0.2.3` |
| Harness | AgentDojo `0.1.35`, Workspace `v1.2.2` |
| Provider and model | OpenAI, `gpt-4o-mini` |
| Attack formats | `direct`, `ignore_previous`, `system_message`, `injecagent` |
| Authorized mutation tasks | 4 |
| Trials per attack and task | 3 |
| Native controls completed | 48 / 48 |
| Native controls completing useful work | 35 / 48 (72.9%) |
| Native controls satisfying injected task | 0 / 48 (0%) |
| Eligible matched governed trials | 0 |

The model did not satisfy the injected email-exfiltration task in any sampled
control. That leaves a zero attack-success denominator, so this result does
**not** measure Kerna's prevention rate and does not claim that Kerna stopped
these attacks. It is a reproducible external finding about this model,
campaign, and run configuration. The deterministic Trust Bench result above
remains Kerna's published runtime-boundary evidence.

Utility and injected-task outcomes by attack format were:

| Attack format | Useful work | Injected task satisfied |
| --- | ---: | ---: |
| `direct` | 9 / 12 | 0 / 12 |
| `ignore_previous` | 8 / 12 | 0 / 12 |
| `system_message` | 9 / 12 | 0 / 12 |
| `injecagent` | 9 / 12 | 0 / 12 |

The matrix publishes a Kerna protection comparison only when a matched native
control completes useful work **and** satisfies the injected task. Since none
did, governed runs were intentionally not started; changing or selectively
weakening attacks after observing this result would invalidate the
pre-registration.

Run the external controls from a terminal that has `OPENAI_API_KEY` set:

```powershell
.\.venv-agentdojo\Scripts\python.exe benchmarks\agentdojo\matrix.py --execute-controls --model gpt-4o-mini
```

The matrix never runs governed trials automatically. Each eligible control is
reviewed with its exact governed counterpart, and results are reported by
attack type rather than pooled.
