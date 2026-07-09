# The Kerna Event Schema

Kerna ensures 100% observability into the agent execution loop through a deterministic, strictly ordered event pipeline. Every action the agent takes, and every decision the runtime makes, is recorded as an `EventTrace`.

## Lifecycle of a Tool Call

When an agent decides to use an MCP tool, Kerna emits the following events in order:

1. **`tool.call.requested`**: The raw intention from the LLM. Includes the tool name and proposed arguments.
2. **`tool.policy.checked`**: The result of the Policy Engine evaluation. Contains the decision (`allow`, `deny`, `require_confirmation`) and the reason.
3. **`budget.checked`**: Verifies that the task has not exceeded its configured limits (`max_tool_calls`, `max_cost_usd`, etc.).
4. **`tool.call.started`**: Emitted immediately before control is handed over to the MCP sub-process.
5. **`tool.call.completed`** / **`tool.call.failed`**: Emitted when the MCP tool returns its result or crashes. Contains execution duration and the final payload.

## Accessing Traces

You can inspect the event trace for any task using its Task ID:

```bash
kerna inspect <task_id>
```

Or quickly view the trace for the most recent run:

```bash
kerna trace last
```

The output gives you a comprehensive, chronological view of exactly what happened, and more importantly, *why* certain actions were blocked or allowed by the Trust Layer.
