# The Scheduler

The Scheduler is the heartbeat of Kerna. It is an autonomous "Self-Correction Loop".

## The Loop

When you run `kerna run "Goal"`, the Scheduler:
1. Creates a new Task in the database.
2. Injects relevant historical context from Memory.
3. Submits the prompt to the LLM.
4. Parses any tool calls returned by the LLM.
5. Checks tool calls against the **Permission Engine**.
6. Executes approved tool calls via the **MCP Registry**.
7. Appends the tool outputs back to the context.
8. Loops back to step 3.

The loop terminates when the LLM returns a final natural language string without any tool calls, signaling the goal is achieved.

## Resiliency

If a tool fails or throws an exception, Kerna catches it, logs it, and passes the error string back to the LLM. The LLM is then prompted to try an alternative approach. 

Kerna tracks the number of retries and will hard-stop execution if `max_tool_rounds` or `max_retries` is hit to prevent infinite loops and runaway API costs.
