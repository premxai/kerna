# Budgets (Execution Guardrails)

Kerna ensures that no agentic task can consume unbounded resources or runaway infinitely. This is achieved using the `BudgetTracker`, which enforces strict execution budgets configured per-task or per-system.

## Budget Configuration

A `BudgetConfig` struct allows administrators to set the following limits:

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `max_runtime_seconds` | `u64` | `300` | The hard timeout for the entire agentic loop. If this is exceeded, Kerna forcibly terminates the task and cleans up MCP processes. |
| `max_tool_calls` | `u64` | `25` | Maximum number of MCP tool calls allowed per task. Prevents infinite tool-calling loops where the agent repeatedly retries failing tools. |
| `max_llm_calls` | `u64` | `10` | Maximum number of turns (messages) with the LLM API. Protects against excessive API usage. |
| `max_cost_usd` | `f64` | `0.25` | Maximum spend allowed for a task. Requires a provider that supports cost-per-token tracking. |
| `max_output_bytes` | `u64` | `50,000` | Limits the size of tool outputs that can be returned to the LLM context. If an MCP server returns huge data, Kerna truncates it to protect the prompt buffer. |
| `max_memory_writes` | `u64` | `20` | Limits the number of times the agent can durable write to the Memory Engine to prevent database poisoning/exhaustion. |

## Budget Checks

During execution, Kerna sinks `budget.checked` events into the trace pipeline *before* executing a tool or invoking the LLM. 

If any budget limit is exceeded, a `budget.exceeded` event is emitted, and the task immediately aborts with an error state. This is a non-recoverable error for the agent.

## Example Configuration

In `kerna.toml`:

```toml
max_runtime_seconds = 120
max_tool_calls = 10
max_llm_calls = 5
max_cost_usd = 0.50
max_output_bytes = 100000
max_memory_writes = 5
```
