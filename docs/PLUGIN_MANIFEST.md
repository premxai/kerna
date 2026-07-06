# Kerna Plugin Manifest (`manifest.toml`)

Plugins running under Kerna MUST declare their required capabilities and metadata in a `manifest.toml` file. If a plugin does not provide this manifest, Kerna runs it under **Legacy Warning** mode, assuming full capability access but flagging the plugin in risk assessments.

## Example Manifest

```toml
name = "github-plugin"
version = "1.0.0"
description = "Interact with GitHub repositories and issues."
author = "Your Name"

[capabilities]
# Explicitly list the capabilities this plugin requires
network_outbound = ["api.github.com", "raw.githubusercontent.com"]
fs_read = [".git/config", "src/"]
fs_write = []
shell = false
env = ["GITHUB_TOKEN"]

[approval]
# Require explicit user approval for specific high-risk actions
require_for = ["github/create_repository", "github/delete_repository"]
```

## Capability Types

- **`network_outbound`**: A list of allowed hostnames. Wildcards (e.g. `*.github.com`) are permitted but discouraged.
- **`fs_read`**: A list of paths the plugin is allowed to read from. Kerna mounts these paths read-only in the sandbox.
- **`fs_write`**: A list of paths the plugin is allowed to write to. Kerna maps these paths with write permissions.
- **`shell`**: Boolean. If true, the plugin is permitted to execute arbitrary shell commands (extremely high risk).
- **`env`**: A list of environment variables the plugin requires. Kerna will only forward these specific variables from the host.

## Enforcement

When a plugin is initialized, Kerna reads the manifest and computes a **Risk Card**. During execution, Kerna enforces these capabilities:
- Network requests are monitored via the gateway layer (if applicable).
- File system access is physically constrained using directory isolation (`sandbox.rs`).
- Unapproved tools trigger an immediate task abort.
