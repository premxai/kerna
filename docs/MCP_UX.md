# MCP UX and Security Operations

Kerna provides first-class support for the Model Context Protocol (MCP), but adds a critical layer of operations, observability, and security on top.

## Adding an MCP Server

You can register an MCP server through the CLI. Kerna will proxy the server via Stdio:

```bash
kerna mcp add mockmcp "path/to/kerna.exe" -- mockmcp
```

## The Risk Card

When you add an MCP server, you are extending the capabilities of the agent. To understand the security implications, you can generate a **Risk Card**.

The Risk Card queries the MCP server for its tools, analyzes them against your configured global policies (`allow_tools` / `deny_tools` / `require_confirmation`), and computes a risk score.

```bash
kerna mcp risk mockmcp
```

Output:
- **Low Risk**: The server has tools, but they are all read-only, or strictly bounded by `deny` rules.
- **Medium Risk**: The server has state-mutating tools that require user confirmation.
- **High Risk**: The server has auto-allowed shell access, dangerous network capabilities, or unvetted scripts.

## Tool Filtering

If an MCP server provides a dangerous tool (e.g., `execute_sql_query`) alongside safe tools (e.g., `list_tables`), you can explicitly block or allow specific tools inside `kerna.toml` using the `allow_tools` and `deny_tools` properties for that specific server.

```toml
[[mcp_servers]]
name = "postgres_mcp"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-postgres", "postgresql://localhost/mydb"]
allow_tools = ["list_tables", "describe_table"]
deny_tools = ["execute_query"]
```
