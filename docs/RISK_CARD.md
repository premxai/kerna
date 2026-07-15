# Kerna MCP risk cards

A risk card is a human-readable review of a connector before you grant it
access. It is not a permission grant and it never overrides Kerna's
fail-closed policy.

## What a risk card shows

For a configured MCP server, `kerna mcp risk <name>` reports:

- discovered tools and the effective allow/deny filters;
- secrets the connector declares, showing only whether an environment variable
  is present—not its value;
- declared network hosts and other manifest metadata;
- tools that the connector contract itself requires approval for.

Curated plugins include a `manifest.toml`. Kerna applies that manifest at
runtime as a narrowing contract: it can restrict callable tools, add approval
requirements, and prevent a configured secret from being forwarded unless the
plugin also declares that exact name. A malformed discovered manifest prevents
the plugin from starting.

Third-party servers without a manifest may still be configured, but they are
unreviewed: inspect their tools, set a narrow `allow_tools` filter, and grant
each tool explicitly before use.

## What actually enforces safety

Risk severity is explanatory. Execution remains governed by four independent,
fail-closed controls:

1. **Manifest contract** — narrows a connector's declared capabilities and
   secret access.
2. **Permission policy** — every tool is `deny`, `require_confirmation`, or
   `auto_approve`; there is no automatic permission based on a risk score.
3. **Budgets and sandboxing** — tool calls, runtime, output, and other resource
   limits bound every task; runtime mode determines the available process and
   network isolation.
4. **Per-action approval** — a tool marked `require_confirmation`, including a
   manifest-required write, pauses for a person or is denied in unattended
   contexts.

## Review a connector

```bash
kerna pack install google-workspace
kerna mcp risk google-calendar
kerna doctor                       # setup state for every configured connector
kerna mcp doctor google-calendar   # local command/configuration diagnostics
kerna mcp probe google-calendar    # start it and list its transport tools
```

Before enabling a connector, confirm its described job, tools, requested
secrets, and network reach match what you intend to use. Then grant the
smallest useful set in `kerna.toml`; leave all other tools denied.
