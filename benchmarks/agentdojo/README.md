# AgentDojo integration

This directory prepares a reproducible, external prompt-injection evaluation
for Kerna. It does **not** yet report an AgentDojo score.

AgentDojo's standard runner calls its in-memory `FunctionsRuntime` directly.
Running that CLI with an OpenAI-compatible Kerna endpoint would test model
routing only; it would bypass Kerna's MCP policy, budget, approval, process
isolation, and receipt path. Such a result must never be presented as
Kerna-governed.

## Readiness check

The check is local, free, and makes no model request:

```bash
python benchmarks/agentdojo/preflight.py
```

To install the pinned benchmark package into an isolated Python environment:

```bash
python -m venv .venv-agentdojo
.venv-agentdojo\Scripts\python -m pip install -r benchmarks/agentdojo/requirements.txt
```

On macOS or Linux, replace `.venv-agentdojo\Scripts\python` with
`.venv-agentdojo/bin/python`.

## Required bridge contract

The actual adapter must be an external MCP plugin, not a benchmark-specific
feature in the Kerna kernel. For each AgentDojo task it must:

1. create the suite environment and expose that task's `FunctionsRuntime`
   functions as MCP tools;
2. let Kerna's normal scheduler discover and call those tools;
3. preserve the AgentDojo environment and function-call trace for scoring;
4. export Kerna's SQLite receipt and the redacted benchmark trace; and
5. score utility and injection security separately with AgentDojo's own task
   checks.

The runner must execute the same suite, model, task subset, attack, policy,
and budget twice: once as the declared control and once through this governed
bridge. Results need explicit trial count, random seed where available, cost,
latency, task utility, unsafe-action prevention, and false-block rate.

## Why a provider key is not requested here

External AgentDojo runs make paid model calls. This repository keeps the
adapter setup and preflight free. Choose a provider, model, task subset, and a
maximum spend before executing the governed comparison.

See [the implementation contract](../../docs/AGENTDOJO_ADAPTER.md) and the
[benchmark methodology](../../docs/BENCHMARKS.md).
