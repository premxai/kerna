# Kerna Trust Bench

Kerna Trust Bench is the deterministic benchmark for the Kerna runtime boundary. It executes existing integration tests against the built-in MockMCP, so it needs no network access, model key, or external service.

It measures whether Kerna preserves the runtime promises that are independent of a model's intelligence:

- allowed work succeeds and leaves a receipt;
- approval-required work pauses before tool execution;
- ungranted access, explicit deny rules, and path traversal are rejected;
- budgets stop unbounded work;
- MCP child-process failures are contained;
- declared secrets remain scoped to their connector; and
- receipts preserve the policy and budget decision chain.

## Run it

From the repository root:

```bash
node benchmarks/kerna-trust/run.mjs
```

The runner writes a JSON report to `reports/kerna-trust/latest.json`. Reports are intentionally ignored by Git because they are generated evidence, not source.

Run one category:

```bash
node benchmarks/kerna-trust/run.mjs --category budget
```

Choose a report path for CI or comparison work:

```bash
node benchmarks/kerna-trust/run.mjs --out reports/kerna-trust/ci.json
```

## Read the score correctly

This is a runtime-governance benchmark, not an LLM capability leaderboard. A perfect score means the deterministic Kerna boundary behaved as specified for these scenarios. It does not prove that an arbitrary model will solve an arbitrary task safely.

External suites such as AgentDojo, τ-bench, ToolSandbox, Terminal-Bench, and SWE-bench will be added through adapters after this core harness remains stable in CI.
