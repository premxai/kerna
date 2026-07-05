# Permissions Engine

Kerna agents operate within a strict, "fail-closed" trust boundary.

By default, an agent cannot do anything. It cannot read your files, access the network, or run terminal commands unless explicitly granted access.

## Declaring Capabilities

Permissions are configured globally in your `kerna.toml` using the `[[permissions]]` table. You map specific tool names (or a wildcard `*`) to an action policy: `auto_approve`, `require_confirmation`, or `deny`.

```toml
# Grant read access automatically
[[permissions]]
tool = "fs.read"
action = "auto_approve"

# Require user confirmation for writing
[[permissions]]
tool = "fs.write"
action = "require_confirmation"

# Deny all other tools
[[permissions]]
tool = "*"
action = "deny"
```

> **Note:** Rule order dictates precedence. Kerna evaluates rules sequentially from top to bottom.

## The Interceptor

The Kerna scheduler intercepts every tool call *before* it reaches the plugin. It validates the request against the `kerna.toml` policy. If an agent attempts an action that requires approval, the runtime pauses and waits for manual user confirmation. If it attempts an undeclared action (or hits a `deny`), it is instantly blocked. Kerna also enforces built-in safety overrides for dangerous tools (e.g., `delete_file`), forcing them to `require_confirmation` regardless of the configuration.
