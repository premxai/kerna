# Research Agent Example

This example demonstrates how to configure Kerna to perform deep web research.

## Prerequisites
You need the `browser` plugin and `filesystem` plugin installed.

## kerna.toml

```toml
llm_provider = "openai"
llm_model = "gpt-4o"
db_path = "kerna.db"

[[mcp_servers]]
name = "browser"
command = "npx"
args = ["-y", "@anthropic/mcp-playwright"]
enabled = true

[[mcp_servers]]
name = "filesystem"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "./reports"]
enabled = true

[[permissions]]
tool = "browser.*"
action = "auto_approve"

[[permissions]]
tool = "fs.write"
action = "require_confirmation"
```

## Running the Agent

Start the runtime and issue the task:

```bash
kerna run "Research the top 5 YC startups hiring AI engineers in SF. Save a summary to ./reports/yc_ai_startups.md"
```

## Observing the Run

Check the timeline and execution trace:
```bash
kerna task list
kerna inspect <task_id>
kerna export <task_id> --format md --out report.md
```
