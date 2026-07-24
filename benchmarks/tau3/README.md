# Kerna tau3 utility pilot

This is the utility lane for the current upstream **tau3** benchmark. The
older `tau-bench` repository is explicitly outdated; the pinned tau3 checkout
is currently `1d244f5dca42944b67a379b44bfeb9f5748f189d`.

## What is pre-registered

The first native calibration is intentionally small and reproducible:

- domain: `retail`
- task IDs: `0`, `1`, `2`
- trials: one per task
- agent and user simulator: `gpt-4o-mini`
- concurrency: one
- max steps: 60
- timeout: 300 seconds per task
- seed: 300

This calibration measures only tau3 agent utility. It must finish and be
reviewed before pre-registering a matched Kerna arm. The first attempted
configuration used `gpt-4.1-nano` with a 20-step limit; all three tasks hit
that ceiling, so it is retained only as an unpublished calibration failure.

## Why a native result is not enough

tau3 executes tool calls by calling its stateful environment directly. Kerna
uses MCP child processes and gates their tool calls. A valid Kerna comparison
therefore needs an adapter that sends the *same* tau3 tool call through
`kerna gateway`, routes it back to the same in-memory tau3 environment, and
records the matching Kerna receipt. Merely pointing tau3's LLM at Kerna's
OpenAI-compatible server would change the agent loop and would not be a fair
utility comparison.

The adapter must satisfy these invariants before governed runs are allowed:

1. Tool names, JSON schemas, task state, task ID, model, user simulator, seed,
   and call limits are identical to the native control.
2. Every agent tool call traverses `kerna gateway` and produces a receipt.
3. The bridge invokes the same tau3 environment instance that is evaluated for
   reward, not a cloned or mocked state.
4. Permissive and governed policies differ only in declared Kerna policy,
   budget, and approval configuration.
5. Raw conversations, API keys, and task payloads remain in ignored reports.

## Free setup and preflight

The setup is stored outside the Git worktree in ignored `reports/tau3-source`.
It needs `uv` and Python 3.12. The preflight sets `PYTHONUTF8=1` because the
current upstream data checker prints Unicode status markers that fail in a
legacy Windows console encoding.

```powershell
python benchmarks\tau3\preflight.py
python benchmarks\tau3\preflight.py --require-provider
```

## Native pilot

Planning makes no API calls:

```powershell
python benchmarks\tau3\run_native.py
```

Only after reviewing the plan and using a terminal where `OPENAI_API_KEY` is
already set:

```powershell
python benchmarks\tau3\run_native.py --execute
```

This creates a native-calibration result only. Do not publish it as a Kerna
result, and do not launch a governed comparison until the gateway adapter
contract above is automated and verified.

## Gateway adapter contract

The adapter is now a local-only transport boundary:

```text
tau3 orchestrator -> kerna gateway -> stdio MCP bridge -> same tau3 environment
```

The bridge listens only on `127.0.0.1`, authenticates with a random per-run
bearer token, and has no provider credential. The runner removes
`OPENAI_API_KEY` before starting Kerna and its MCP child. Tau3 keeps ownership
of task state and evaluation; the adapter replaces only direct tool execution.

Run the free contract before any provider-backed run:

```powershell
python benchmarks\tau3\gateway_contract_test.py
```

It checks schema passthrough, same-state tool execution, fail-closed unknown
tools, and requested, policy-checked, completed, and blocked receipt events.

## First matched gateway trial

The corrected native calibration completed only retail task `0`. Under the
pre-registered rule, tasks `1` and `2` are not eligible for a Kerna
counterpart, so the first bounded Kerna trial is task `0` only. It is one
matched utility observation, not a launch claim or stable percentage.

With `OPENAI_API_KEY` already set in your terminal:

```powershell
python benchmarks\tau3\run_gateway.py --execute
```

The runner uses tau3's pinned `uv` environment automatically. It creates an
ignored raw result and a redacted wrapper at
`reports/tau3/gateway-task-0.json`. Do not publish it until the native and
gateway outputs are reviewed together and more trials are pre-registered.

## Replication campaign

One native success and one gateway outcome are observations, not a benchmark
comparison. The campaign runs 20 pre-registered native controls for task `0`,
using seeds `1000` through `1019`. A Kerna counterpart is run only for an
exact native control that receives reward `1.0`; failed controls are retained
and reported, never retried into a pooled result.

Run the phases separately, reviewing native eligibility before provider spend
on the gateway arm:

```powershell
python benchmarks\tau3\campaign.py
python benchmarks\tau3\campaign.py --execute-controls
python benchmarks\tau3\campaign.py --execute-governed
```

All campaign outputs remain under ignored `reports/tau3/campaigns/`. A reviewed
aggregate must publish raw counts, control completion, governed retention,
receipt coverage, blocks, cost, latency, the pinned revisions, and the
limitation that this is one model and one retail task family.
