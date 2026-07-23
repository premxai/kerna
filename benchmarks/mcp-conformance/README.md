# Kerna MCP conformance

This runner executes the two official core client scenarios from
`@modelcontextprotocol/conformance@0.1.16`:

- `initialize`
- `tools_call`

Kerna's product boundary is an isolated **stdio** child process. The official
conformance framework serves client scenarios over local streamable HTTP, so
the hidden benchmark command starts pinned `mcp-remote@0.1.38` as the
untrusted child-process bridge. The bridge is test-only and does not add a
remote HTTP transport to Kerna's production configuration model.

Run locally:

    node benchmarks/mcp-conformance/run.mjs --out reports/mcp-conformance/latest

The result proves that Kerna's stdio client completes the official 2025-06-18
initialization and tools-call semantics through the bridge. It does not claim
support for later MCP features such as OAuth, SSE reconnection, elicitation,
resources, or prompts. Those require explicit product support and separate
conformance scenarios before being advertised.
