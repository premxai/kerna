# Architecture

Kerna is built on a clean, decoupled architecture where the runtime is completely isolated from the execution tools.

## The Monolith

The Kerna runtime itself is a single Rust binary (`kerna.exe`). It is composed of four primary subsystems:

1. **The CLI / Shell**: Handles the interactive REPL, session management, and observability commands (`inspect`, `explain`, `export`).
2. **The Scheduler (`scheduler.rs`)**: The core autonomous loop. It receives a goal, retrieves context from memory, prompts the LLM, and parses the resulting tool calls. It handles retries, self-correction, and tool errors.
3. **The Memory Engine (`memory.rs`)**: An embedded SQLite database that tracks everything. It stores tasks, session metadata, episodic memory (with vector embeddings), knowledge graphs, and raw execution logs. This is what powers Kerna's deep observability.
4. **The Permission Engine (`permissions.rs`)**: The trust boundary. It intersects every single tool call to ensure the agent is not violating the `kerna.toml` policies.

## The MCP Plugin Boundary

Kerna does not natively know how to browse the web or edit your filesystem. 
Instead, it speaks the **Model Context Protocol (MCP)**. 

Tools are provided by independent plugin binaries. 
The Scheduler communicates with these plugins over `stdio`. 
If a plugin crashes, the Kerna runtime survives. If a plugin goes rogue, the Permission Engine blocks it based on the explicitly declared `capabilities` in `kerna.toml`.

```text
                User
                  │
          kerna / CLI
                  │
          Runtime (Rust)
    ┌──────────┬──────────┬───────────┐
    │          │          │
 Scheduler   Memory    Permission
    │          │          │
    └──────────┼──────────┘
               │
        Tool Runtime (MCP)
               │
      Browser / Files / GitHub
```
