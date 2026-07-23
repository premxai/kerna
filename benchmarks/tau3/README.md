# Kerna tau3 utility pilot

This is the utility lane for the current upstream **tau3** benchmark. The
older `tau-bench` repository is explicitly outdated; the pinned tau3 checkout
is currently `1d244f5dca42944b67a379b44bfeb9f5748f189d`.

## What is pre-registered

The first native control is intentionally small and reproducible:

- domain: `retail`
- task IDs: `0`, `1`, `2`
- trials: one per task
- agent and user simulator: `gpt-4.1-nano`
- concurrency: one
- max steps: 20
- timeout: 300 seconds per task
- seed: 300

This control measures only tau3 agent utility. It must finish and be reviewed
before launching a matched Kerna arm.

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

This creates a native-control result only. Do not publish it as a Kerna result,
and do not launch a governed comparison until the gateway adapter contract
above is automated and verified.
