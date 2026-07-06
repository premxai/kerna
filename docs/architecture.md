# Kerna Architecture

This document defines the architectural philosophy, boundaries, and technical lifecycle of Kerna. It serves as the source of truth for what Kerna is, what it owns, and what it explicitly ignores.

## 1. Why Kerna Exists

Most AI agent frameworks are built as end-user applications (like chat interfaces or coding tools). They execute with full system privileges, treat memory as a temporary buffer, and rely on sprawling, tightly-coupled Python codebases that are difficult to observe or debug in production.

**Kerna is not an application. It is a runtime.**

Kerna exists to provide a secure, observable, and persistent "operating system" layer for autonomous AI agents. It acts as the secure sandbox and orchestration engine, allowing developers to safely run agents on local machines or in the cloud without fearing destructive commands or losing context between sessions.

## 2. What Kerna Owns

Kerna is strictly responsible for the orchestration and safety of the agent loop. It owns:

*   **The Scheduler:** The orchestrator that executes the agent loop, enforces timeouts, and issues commands.
*   **The Memory Engine:** The embedded SQLite database storing the full event trace schema.
*   **Budgets:** The strict constraints (`max_tool_calls`, `max_runtime_seconds`, etc.) that prevent runaway execution.
*   **Permissions & Manifests:** The fail-closed security boundary evaluating `manifest.toml` requirements against actual calls.
*   **Observability:** The recording of exact JSON payloads, tool traces, and thought processes into structured tables for human debugging.
*   **Process Isolation:** Spawning and safely reaping external tool processes (MCP plugins).

## 3. What Kerna Explicitly Does NOT Own

Kerna does **not** own domain logic. 

Kerna does not know how to send an email, book a flight, scrape a website, or compile code. It intentionally delegates all domain-specific logic to external **Model Context Protocol (MCP)** plugins. If replacing a plugin changes how Kerna operates, then Kerna is violating its architectural boundary.

Kerna's responsibility ends at: *"Did I securely spawn the tool, validate the permissions, capture the output, and continue the loop?"*

## 4. The Lifecycle of a Task

When a user executes `kerna run <goal>`, the following strict lifecycle occurs:

1.  **Memory Injection:** The Scheduler queries the SQLite Memory Engine for semantically relevant past tasks and injects them into the system prompt.
2.  **Tool Discovery:** Kerna spawns the MCP servers defined in `kerna.toml` and asks them for their available tools.
3.  **The LLM Loop:** Kerna sends the goal, memory, and tool schema to the LLM.
4.  **Action Intent:** The LLM returns a JSON intention to use a tool.
5.  **Permission Validation (Fail-Closed):** Before the tool is invoked, Kerna intercepts the request and checks `kerna.toml`. If the capability is not explicitly granted, the action is denied immediately.
6.  **Tool Invocation:** Kerna sends the request to the isolated MCP child process.
7.  **Output Capture & Truncation:** The plugin returns the result. If it exceeds token limits, Kerna truncates it to prevent context overflow.
8.  **Commit to Memory:** The reasoning and action are written to the SQLite database.
9.  **Loop Continuation:** Kerna sends the result back to the LLM until the LLM signals the task is complete.

## 5. Plugin Interaction & Process Isolation

Kerna interacts with tools exclusively via the **Model Context Protocol (MCP)** over `stdio`. 

Plugins are treated as untrusted child processes. Kerna spawns them in an isolated working directory (`./sandbox`). If a plugin hangs or times out, Kerna autonomously issues a `SIGKILL`, reaps the zombie process, records the failure, and allows the LLM to choose an alternative path.

## 6. The Security Model

Kerna operates on a **fail-closed** security model. 

By default, an agent has zero privileges. It cannot read the filesystem, access the network, or run bash commands. To grant an agent access to a tool, the user must explicitly list the required capabilities in `kerna.toml`. Furthermore, dangerous capabilities (like `fs.delete` or `shell.exec`) can be flagged with `approval_required`, causing Kerna to pause the execution loop and request human confirmation before proceeding.

## 7. The Memory Model

Memory is not an afterthought in Kerna; it is a native primitive.

Kerna embeds a high-performance SQLite database (`kerna.db`) directly into the Rust binary, utilizing WAL (Write-Ahead Logging) for concurrent access.
*   **Episodic Memory:** Every tool call, LLM prompt, and JSON payload is permanently recorded to disk, allowing engineers to replay or inspect a task exactly as it happened.
*   **Semantic Memory:** Final task summaries are stored. When a new task begins, Kerna automatically retrieves relevant past summaries to provide the agent with continuous, cross-session context.

## 8. Long-Term Roadmap

The v0.1.0-alpha establishes the single-node runtime. Future development will focus on scaling this foundation:

*   **Event Bus:** Transitioning from linear loops to an event-driven architecture, allowing agents to react asynchronously to system events (e.g., file changes, webhooks).
*   **Time-Travel Replay:** The ability to pause a failed task, edit the LLM's prompt in the SQLite DB, and resume the task from the exact point of failure.
*   **Cloud & Distributed Worker Nodes:** Syncing SQLite memory across devices and allowing the Kerna daemon to offload intensive tasks to remote worker nodes over a secure protocol.
*   **Native Plugin SDK:** Providing a streamlined Rust SDK to allow developers to rapidly build safe, compiled MCP plugins to extend Kerna's ecosystem.
