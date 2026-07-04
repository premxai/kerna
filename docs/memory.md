# Memory Engine

Kerna runs on a persistent SQLite embedded database. This engine is the bedrock of Kerna's observability and context injection.

## Schema Architecture

Kerna's database tracks:
- **Tasks:** Every goal you assign the agent.
- **Sessions:** Logical groupings of tasks.
- **Logs:** A granular event stream of exactly what tools the agent called and what errors it encountered.
- **Episodic Memory:** The semantic output of previous tasks. 

## Context Injection

When an agent begins a task, it doesn't start completely blank. The Scheduler queries the Memory Engine for recent semantic memories relevant to the goal and injects them into the prompt. 

This means if the agent learned about a codebase structure in a previous session, it can recall it natively in the current session without manual prompting.

## Observability

Because Kerna logs every action to SQLite, the entire execution is reproducible. This powers:
- `kerna inspect`: View timings, cost, and tokens used.
- `kerna explain`: Parses the logs into a human-readable reasoning chain.
- `kerna export`: Dumps the entire task lifecycle to JSON or Markdown.
