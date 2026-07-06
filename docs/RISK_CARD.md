# Kerna Risk Cards

When a plugin is loaded, Kerna calculates a **Risk Card** which serves as a security passport for the plugin's execution session.

## Risk Score Calculation

The risk score is a float from `0.0` (fully benign) to `10.0` (critical risk). It is calculated based on the capabilities requested in the `manifest.toml`.

- **Baseline**: `0.0`
- **Missing Manifest**: `+10.0` (Legacy Warning Mode)
- **Filesystem Access**:
  - `fs_read` requested: `+1.0`
  - `fs_write` requested: `+3.0`
  - `fs_write` to sensitive path (e.g., `/etc`, `.ssh`, `~`): `+5.0`
- **Network Access**:
  - `network_outbound` requested: `+2.0`
  - `network_outbound` with wildcards (`*`): `+4.0`
- **Execution Access**:
  - `shell` requested: `+10.0`
- **Environment Access**:
  - `env` requested: `+1.0`
  - `env` requesting secrets (`*_KEY`, `*_TOKEN`, `PASSWORD`): `+5.0`

## Risk Tiers

| Score | Tier | Behavior |
|-------|------|----------|
| `0.0 - 2.0` | **Low** | Automatically approved for execution. |
| `2.1 - 6.0` | **Moderate** | Executes, but sensitive operations may require explicit user approval (Converse Mode). |
| `6.1 - 9.9` | **High** | Warns the user on initialization. Enforces strict budgets. |
| `10.0+` | **Critical** | Plugin cannot be loaded unless the user explicitly forces it with `--trust-all` or runs in Legacy Warning Mode. |

## Viewing a Risk Card

You can inspect a plugin's Risk Card before installing it:

```bash
kerna plugin inspect ./path/to/plugin/manifest.toml
```

**Output Example**:
```text
Plugin: github-plugin (v1.0.0)
Risk Score: 4.0 [MODERATE]

Capabilities:
- fs_read: [".git/config", "src/"]
- network_outbound: ["api.github.com"]

Warnings:
- Network outbound requests are permitted.
```
