# Permissions Engine

Kerna agents operate within a strict, "fail-closed" trust boundary.

By default, an agent cannot do anything. It cannot read your files, access the network, or run terminal commands unless explicitly granted access.

## Declaring Capabilities

When you add a plugin to `kerna.toml`, you must explicitly declare its capabilities:

```toml
[[plugins]]
name = "filesystem"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "./"]
enabled = true

# The permissions this plugin is allowed to request
capabilities = ["fs.read", "fs.write"]

# The paths it is restricted to
allowed_paths = ["./"]

# Actions that will hard-pause execution and prompt the user for 'Y/N' approval
approval_required = ["fs.write", "fs.delete"]
```

## The Interceptor

The Kerna scheduler intercepts every tool call *before* it reaches the plugin. It validates the request against the `kerna.toml` policy. If an agent attempts an action that requires approval, the runtime pauses and waits for manual user confirmation. If it attempts an undeclared action, it is instantly blocked.
