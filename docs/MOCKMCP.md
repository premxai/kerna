# MockMCP Deterministic Testing Server

Kerna includes a built-in MCP server called **MockMCP** designed exclusively for CI/CD and trust-layer validation. MockMCP does not contact external services; it deterministically returns predefined responses based on the tool called.

## Purpose

MockMCP allows us to validate the Kerna Runtime Trust Layer (budgets, memory limits, process timeouts, fail-closed policy) without flakiness or external dependencies. 

## Running MockMCP

You can run MockMCP directly using the Kerna CLI:

```bash
kerna mockmcp --mode normal
```

By default, it listens on stdio using JSON-RPC, perfectly matching the MCP protocol.

## Available Tools

When queried with `tools/list`, MockMCP exposes:

1. **`echo`**: Returns the input string. Validates happy path.
2. **`hang`**: Sleeps for 30 seconds before returning. Validates timeouts.
3. **`huge_output`**: Returns 2MB of text. Validates `max_output_bytes` truncation and memory limits.
4. **`invalid_json`**: Returns a malformed JSON-RPC string. Validates Kerna's parser resilience.
5. **`fail_once_then_pass`**: Returns an error on the first call, success on the second. Validates retry logic.
6. **`malicious`**: Attempts to execute an unapproved action.

## Modes

- `--mode normal`: Standard testing mode.
- `--mode malicious`: The server attempts to send arbitrary stdout spam, write to its current directory, and ignore termination signals. This validates Kerna's watchdog and sandbox boundaries.
