# Kerna Event Schema & Sinking

Kerna's observability layer guarantees that every action taken by the agent, plugin, or system is durably recorded before it has side effects.

## Event Taxonomy

All events follow a unified schema. Events are divided into namespaces:

### `tool.*` (Execution Events)
- `tool.call.requested`: Emitted when the LLM asks to use a tool.
- `tool.policy.checked`: Emitted after policy evaluation. If rejected, the task may abort.
- `tool.call.started`: Emitted immediately prior to launching the MCP process or sending the request.
- `tool.call.completed`: Emitted when the tool returns successfully, containing truncated output.
- `tool.call.failed`: Emitted on timeouts, parser errors, or non-zero exit codes.

### `budget.*` (Constraint Events)
- `budget.checked`: Emitted to confirm a budget check passed.
- `budget.exceeded`: Emitted when a budget threshold is violated (aborts task).

### `memory.*` (State Events)
- `memory.read`: Emitted when retrieving memories.
- `memory.write`: Emitted when writing durable facts to the SQLite database.
- `memory.write.skipped`: Emitted if memory writes exceed budget.

## Required Event Fields

```json
{
  "event_id": "evt_...",
  "task_id": "task_...",
  "session_id": "ses_...",
  "timestamp": "2024-05-18T10:00:00Z",
  "event_type": "tool.call.requested",
  "sequence": 1,
  "severity": "info",
  "redaction_status": "none",
  "actor": "llm",
  "model": "claude-3-haiku",
  "tool": "mcp:filesystem/read",
  "policy_decision": "allow",
  "risk_score": 0.0,
  "budget_snapshot_json": "{...}",
  "payload_json": "{...}"
}
```

## Redaction Rules

To comply with enterprise requirements, Kerna redacts sensitive data before writing to the database:
1. **API Keys**: Filtered from `payload_json` if detected (e.g. `sk-...`).
2. **PII**: Configurable regex-based filtering can be applied via Kerna Config.
3. **Payload Truncation**: Tool outputs are hard-capped by `config.max_output_bytes`.

Events modified by these rules have their `redaction_status` set to `redacted`.
