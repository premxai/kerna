# Kerna Plugin Manifest (`manifest.toml`)

Each curated Kerna MCP plugin includes a `manifest.toml` beside its entrypoint.
The manifest is a **runtime contract**: it declares the MCP tools the plugin is
allowed to expose, operations that need confirmation, and environment-variable
names it may receive. Kerna applies those declarations as restrictions on the
effective runtime configuration.

A manifest is found beside a configured entrypoint first, then in Kerna's
shipped `plugins/<name>_mcp/` or `plugins/<name>/` directories. A discovered
manifest that cannot be parsed prevents that plugin from starting; it is never
silently treated as legacy.

## Example

```toml
[plugin]
name = "calendar"
version = "1.0.0"
kind = "tool.mcp"
entrypoint = "mcp_server.py"
source = "kerna-starter-pack"
trust = "verified"

# These are MCP tool names, not broad OS privileges.
capabilities = ["list_events", "add_event"]
requires_approval = ["add_event"]

# Names only. Values remain in the host environment and are never serialized
# into `kerna.toml` or displayed by Kerna.
secrets = ["CALENDAR_TOKEN"]

network_allowlist = ["calendar.example.com"]
declared_outputs = ["text"]
max_output_bytes = 50000
```

## Enforcement semantics

For a plugin with a manifest:

- The effective `capabilities` and `allow_tools` are the intersection of the
  user's configuration and the manifest declaration. If the user leaves either
  list empty, the manifest list becomes the limit. Configuration cannot expand
  the declared tool set.
- Manifest `requires_approval` values are added to the server's approval list.
  They can only add confirmation gates.
- A secret is forwarded only when it appears in **both** the configured server
  entry and the manifest. This requires explicit user configuration and plugin
  disclosure.
- A manifest with no declared tool capabilities blocks all tool calls. This is
  suitable for a resource-only server and avoids accidental empty-list grants.
- `deny_tools`, global permissions, budget limits, folder boundaries, and other
  runtime policy still apply after the manifest contract.

The manifest is not a sandbox. Its network, allowed-path, output, and trust
metadata are shown in risk assessment and guide policy, but a native child
process is not made OS-safe merely by declaring those values. Use a hardened
runtime mode where available and grant only reviewed plugins.

## Legacy plugins

Third-party MCP servers without a manifest remain supported for compatibility,
but Kerna marks them as **Legacy Warning** and relies entirely on explicit
`kerna.toml` policy, filters, budgets, and the selected runtime boundary. Treat
them as unreviewed until they have a tested manifest and connector record.

## Author checklist

1. Put `manifest.toml` next to the plugin entrypoint.
2. List every tool the server may expose under `capabilities`.
3. List every state-changing or externally consequential tool under
   `requires_approval`.
4. Declare only the environment-variable names genuinely needed.
5. Document network destinations and output types where applicable.
6. Test the plugin against malformed requests, denial, timeout, cancellation,
   and output-size limits before publishing it.
