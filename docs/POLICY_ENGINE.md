# The Kerna Policy Engine

Kerna’s Policy Engine intercepts all actions proposed by an agent *before* they are executed, ensuring they comply with your security rules.

## Core Concepts

The Policy Engine checks every proposed tool execution against a set of rules defined in `kerna.toml`.

Rules have two main components:
1. **Target**: Which tool or MCP the rule applies to (e.g., `shell.exec`, `*`).
2. **Action**: What should happen when the tool is requested (`allow`, `deny`, `require_confirmation`).

## Defining Rules

By default, Kerna operates in a **fail-closed** mode. If a tool is not explicitly allowed, it will be denied.

You can configure policies in your `kerna.toml`:

```toml
[[permissions]]
tool = "shell.exec"
action = "require_confirmation"

[[permissions]]
tool = "fs.read"
action = "allow"

[[permissions]]
tool = "*"
action = "deny"
```

## Simulating Policies

To verify that your policies are correctly configured without running a live agent, you can use the built-in simulator:

```bash
kerna policy simulate "shell.exec" '{"command": "rm -rf /"}'
```

This will run the proposed action through the policy engine and print the evaluation trace, showing you exactly which rule matched and the final decision.
