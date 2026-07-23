# Kerna BFCL provider-compatibility pilot

This lane uses the Berkeley Function Calling Leaderboard (BFCL) to establish a
small, reproducible **provider/model native function-calling baseline**. It
measures whether one named model can select a function and produce valid
arguments for ten non-live `simple_python` cases.

It does not measure Kerna policy enforcement, safety, runtime performance, or
agent utility. Kerna already reports its MCP boundary separately. A BFCL score
must never be described as proof that Kerna made a model more capable or safer.

## What is pinned

- Upstream package: `bfcl-eval==2025.12.17`
- Default model: `gpt-4.1-nano-2025-04-14-FC`
- Category: `simple_python`
- Pilot size: 10 fixed case IDs in `pilot-ids.json`
- API concurrency: one request at a time
- Live-data categories: excluded

The official runner's package name matters: `bfcl-eval` is the maintained
Berkeley evaluator. Do not install the unrelated `bfcl` PyPI project.

## Install once

From the repository root in PowerShell:

```powershell
python -m venv .venv-bfcl
.\.venv-bfcl\Scripts\python.exe -m pip install --upgrade pip
.\.venv-bfcl\Scripts\python.exe -m pip install -r benchmarks\bfcl\requirements.txt
```

BFCL includes model handlers and evaluator dependencies for many providers, so
this initial install can be substantial. The virtual environment and raw runs
are ignored by Git.

## Free preflight

This checks the package version, CLI, fixed pilot fixture, and optionally the
presence of a credential. It makes no model or network inference calls and
never prints a credential.

```powershell
.\.venv-bfcl\Scripts\python.exe benchmarks\bfcl\preflight.py
.\.venv-bfcl\Scripts\python.exe benchmarks\bfcl\preflight.py --require-provider
```

## Plan, then execute

The first command writes the exact bounded plan without sending any provider
requests:

```powershell
.\.venv-bfcl\Scripts\python.exe benchmarks\bfcl\run.py --model gpt-4.1-nano-2025-04-14-FC
```

After setting a provider-side spending cap and confirming `OPENAI_API_KEY` is
already present in that same terminal, execute the ten-case pilot:

```powershell
.\.venv-bfcl\Scripts\python.exe benchmarks\bfcl\run.py --execute --model gpt-4.1-nano-2025-04-14-FC
```

The runner uses one API request at a time, writes raw BFCL output only under
ignored `reports/bfcl/runs/`, and writes a redacted aggregate to
`reports/bfcl/latest.json`. Review the aggregate, the exact package version,
model snapshot, pilot case IDs, and score files before creating a committed
public result. A partial ten-case score is a pilot, not an official BFCL
leaderboard score.
