# Kerna Killer Demo Script

This script demonstrates the unique value proposition of Kerna: observing, bounding, and securing agent execution.

## Setup

```bash
kerna init --quick
kerna provider add openai --provider-type openai --api-key-env OPENAI_API_KEY
kerna mcp add mockmcp "/path/to/kerna" -- mockmcp
```

## Act 1: The Fast Path Deny

Show how Kerna intercepts dangerous actions before they even reach the agent's LLM budget.

```bash
kerna policy simulate shell.exec "{\"command\": \"rm -rf /\"}"
```
**Point out:** The simulator instantly denies the action based on the global fail-closed policy.

## Act 2: The Risk Card

Demonstrate how Kerna analyzes external plugins for threats.

```bash
kerna mcp risk mockmcp
```
**Point out:** Kerna automatically scans the 11 registered tools and summarizes the security posture without running any code.

## Act 3: The Observable Trace

Execute a task that intentionally fails due to constraints or lack of API keys, proving the execution guardrails.

```bash
kerna run "Use MockMCP echo and explain what happened"
```

Then immediately run:
```bash
kerna trace last
```

**Point out:** Every step is logged chronologically, from `tool.call.requested` to the final `budget.checked` or `tool.policy.checked` decision, proving 100% observability.
