# Kerna Benchmark Methodology

Kerna is a runtime trust layer, not an LLM. Its evaluation must therefore measure two independent properties:

1. whether an agent completes useful work; and
2. whether the runtime enforces policy, budgets, isolation, and observability while that work happens.

Kerna never combines those properties into one score. A high task-success result cannot offset an unsafe action. The detailed execution and publication rules are in [BENCHMARK_EXECUTION_PLAN.md](BENCHMARK_EXECUTION_PLAN.md). The full product scorecards, comparison contract, and run order are in [BENCHMARK_PROGRAM.md](BENCHMARK_PROGRAM.md).

## Kerna Trust Bench

[`benchmarks/kerna-trust`](../benchmarks/kerna-trust) is the required deterministic regression suite. It uses the built-in MockMCP and mock provider, needs no API key, and produces a machine-readable JSON report.

The required scorecard is:

| Metric | Definition |
| --- | --- |
| Scenario pass rate | Fraction of deterministic runtime scenarios that pass. Required: 100%. |
| Unsafe-action prevention | Denied tools and out-of-bound actions do not execute. Required: 100%. |
| Approval ordering | Approval-required tools do not start before a recorded approval. Required: 100%. |
| Budget enforcement | Exhausted budgets abort or skip work as designed. Required: 100%. |
| Isolation | Plugin secret scope, read-only folder boundaries, malformed responses, oversized payloads, and hangs are contained. Required: 100%. |
| MCP protocol boundary | Stdout noise is tolerated; unrelated response IDs and duplicate tool declarations are bounded or rejected. Required: 100%. |
| Receipt integrity | Policy and budget events appear in the expected order. Required: 100%. |

The benchmark runs on every pull request through CI. It is intentionally deterministic so that a failure is a regression to investigate, not a model-quality fluctuation.

The adversarial protocol cases run the `kerna mockmcp` binary as a separate
stdio child process. They exercise Kerna's real MCP client and registry path,
not an in-memory mock or a network service.

## External benchmark roadmap

The [AgentDojo adapter contract](AGENTDOJO_ADAPTER.md) has a local no-cost
preflight and task-scoped MCP bridge in
[`benchmarks/agentdojo`](../benchmarks/agentdojo). It intentionally publishes
no score until a matched native control and Kerna-governed model run produces
verifiable Kerna receipts for the governed path.

| Suite | Purpose | Kerna adapter requirement |
| --- | --- | --- |
| AgentDojo | Prompt-injection attacks against tool-using agents | Compare AgentDojo's native control with the same tools routed through Kerna; measure task success, unsafe-action prevention, and false blocks. |
| τ-bench / ToolSandbox | Stateful, policy-constrained tool interaction | Translate benchmark tools into MCP servers or a compatible gateway adapter. |
| Terminal-Bench | Real terminal tasks | Expose the terminal harness through a tightly scoped Kerna MCP connector. |
| SWE-bench | Real GitHub issue resolution | Place a coding-agent tool harness behind Kerna and score patch success separately from governance outcomes. |
| OSWorld / WebArena | Desktop and browser use | Use only when desktop or browser MCP workflows are in the evaluated product scope. |

## Evaluation protocol

For external suites, publish the following with every result:

- Kerna version and Git commit;
- benchmark version and task subset;
- model, provider, and agent-harness version;
- the exact policy and budget configuration;
- trial count and random seed where supported;
- task success, unsafe-action prevention, false-block rate, cost, and latency; and
- redacted result artifacts only. Never publish credentials, raw private prompts, or sensitive tool payloads.

Compare the same model and agent harness in at least two configurations:

1. a permissive control configuration; and
2. the Kerna governed configuration.

This shows the practical trade-off between task success, cost, latency, and safety controls without claiming that a runtime changes the underlying model's intelligence.
