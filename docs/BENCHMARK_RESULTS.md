# Kerna benchmark results

This page reports released benchmark evidence. Results are split by what they
actually measure; no utility or safety result is combined into a single score.

## Kerna Trust Bench v0.1.0

**Status:** passed

| Field | Result |
| --- | --- |
| Source revision | `8026567` |
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
