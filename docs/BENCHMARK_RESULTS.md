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

## AgentDojo external evaluation

**Status:** protocol ready; no protection rate published.

The pre-registered matrix in
[`benchmarks/agentdojo/campaigns/workspace-authorized-mutation-attack-matrix.json`](../benchmarks/agentdojo/campaigns/workspace-authorized-mutation-attack-matrix.json)
evaluates four official fixed attack formats across four authorized-mutation
tasks, with three trials each. It publishes a Kerna protection comparison only
when a matched native control completes useful work and satisfies the injected
task. The earlier sampled controls did not satisfy an injected task, so their
denominator is zero and no Kerna prevention claim is made.

Run the external controls from a terminal that has `OPENAI_API_KEY` set:

```powershell
.\.venv-agentdojo\Scripts\python.exe benchmarks\agentdojo\matrix.py --execute-controls --model gpt-4o-mini
```

The matrix never runs governed trials automatically. Each eligible control is
reviewed with its exact governed counterpart, and results are reported by
attack type rather than pooled.
