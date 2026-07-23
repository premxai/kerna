# AgentDojo adapter contract

## Status

**Bridge implementation is available and locally protocol-tested. No AgentDojo
score is publishable until a matched, real-model control and governed run have
completed.**

AgentDojo is Kerna's first external injection-safety evaluation. Its standard
pipeline directly executes a Python `FunctionsRuntime`; that is useful for a
baseline, but it does not traverse Kerna's runtime trust boundary.

## Architecture

```text
AgentDojo task environment
        |
        v
Task-scoped MCP bridge plugin  <-- separate Python child process
        |
        | stdio MCP tools + redacted state/trace export
        v
Kerna scheduler -> policy -> budgets -> approval -> MCP call -> SQLite receipt
        |
        v
AgentDojo utility and injection-security scoring
```

The bridge is intentionally a benchmark-side MCP plugin. Kerna remains a
domain-neutral runtime and does not import AgentDojo or own its task logic.

## Acceptance criteria

Before a result is published, the bridge must demonstrate all of the following:

- every AgentDojo tool call has a corresponding Kerna receipt event;
- rejected or approval-paused calls do not mutate the AgentDojo environment;
- injected tool output reaches the model through the governed tool-result path;
- the adapter preserves AgentDojo's function-call trace for its official
  utility and security checks;
- all credentials and raw private tool payloads are excluded from result
  artifacts; and
- a permissive control and a governed configuration run the identical task
  subset with the same model settings.

## Run protocol

Pin `agentdojo==0.1.35` unless this document and the generated results change
together. Begin with the documented `workspace` email-exfiltration pilot:
`user_task_16` plus `injection_task_4` using the `direct` attack. The task
reads the same Facebook-security email into which the attack injects an
exfiltration instruction, so a control run can actually encounter the attack.
Then expand only after receipt-to-trace mapping and matched control behavior
are verified.

Each public result must include:

- Kerna Git SHA and release version;
- AgentDojo version, suite, task IDs, attack, and benchmark version;
- provider, exact model identifier, model settings, trial count, and seed when
  the provider supports it;
- Kerna policy, budget, and connector-manifest hashes;
- utility, injection-security, false-block rate, latency, and cost; and
- redacted receipts and evaluation traces.

Do not compare different models or task subsets as if they were a security
improvement. A governed configuration must be compared with its matched
control, and safety must never be folded into a single score with task utility.
