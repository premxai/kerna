# Cross-IDE and CLI Runtime

Kerna is an MCP gateway for clients such as Codex CLI, Claude Code, and any IDE
that accepts a standard MCP server configuration. It governs only MCP tool calls
routed to `kerna gateway`; a client's native editor, terminal, and built-in
tools remain outside this boundary.

The gateway uses the official Rust MCP SDK (`rmcp`) for stdio framing,
capability negotiation, and protocol-version negotiation. Its supported client
revisions include `2024-11-05`, `2025-03-26`, `2025-06-18` (Codex), and
`2025-11-25`; source builds require Rust 1.88 or newer.

## Production plugin contract

Production plugins are OCI images that speak MCP over stdio through their image
entrypoint. Native commands are not accepted by `kerna gateway`.

```toml
[[mcp_servers]]
name = "reviewed-files"
runtime_mode = "docker"
image = "registry.example/reviewed-files@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
manifest_path = "plugins/reviewed-files/manifest.toml"
manifest_sha256 = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
signing_public_key = "BASE64_ED25519_PUBLIC_KEY"
read_roots = ["docs"]
write_roots = ["patches"]
```

The reviewed manifest must have a base64 Ed25519 `signature`. Kerna verifies
the signature and pinned manifest hash before it starts the image. Containers
run with a read-only root filesystem, no capabilities, no network, no Docker
socket, a bounded `/tmp`, and only the declared project mounts. Read and write
roots are project-relative, existing, symlink-safe, and disjoint.

## Client setup

Kerna never edits a client configuration automatically. Print and review the
project configuration instead:

```powershell
kerna client config --client codex --workspace .
kerna client config --client claude-code --workspace .
kerna client config --client qoder --workspace .
kerna client config --client generic --workspace .
```

Every adapter launches `kerna gateway --workspace <project>` directly.
`generic` emits portable MCP JSON; each named adapter emits only its reviewed
native configuration shape. Validate a printed configuration and a real
`initialize → tools/list` gateway handshake without modifying client settings:

```powershell
kerna client doctor --client qoder --workspace .
```

## Local dashboard

The demo surface is a local browser dashboard backed by the same SQLite state
as the gateway. It binds only to loopback and does not expose remote control.

```powershell
kerna dashboard --workspace .
```

It shows active MCP clients, the last-minute decision and latency counters,
contained plugin mounts, approvals, receipts, and model-routing state. The SSE
stream starts with a redacted snapshot and updates within one second after
durable state changes. Approval buttons require the dashboard's per-launch
CSRF token and the exact `127.0.0.1:<port>` origin.

## Local-model registry and routing

Kerna vendors a small pinned, provenance-carrying subset of
[`0xSero/local-ai-registry`](https://github.com/0xSero/local-ai-registry).
It is a recommendation/verification catalog only: it never pulls model
artifacts, Docker images, or launch commands.

```powershell
kerna models detect
kerna models list
kerna models recommend --purpose coding
kerna models verify --provider ollama
```

The detector reads NVIDIA VRAM on Windows/Linux and Apple unified memory on
macOS. A device that has no matching validated recipe receives no validated
recommendation. `kerna run --privacy local-only` remains fail-closed: it must
resolve a local route and find the exact selected model on that local runtime.
Models explicitly labelled `:cloud` by a local runtime are refused for
`local-only`, even though that runtime lists them in its inventory.
Codex, Claude Code, Qoder, and other external clients choose their own model;
Kerna displays them as **external client / model not controlled** and governs
only the MCP calls routed through its gateway.

For another device, pass an explicit JSON `HardwareProfile` using
`kerna models recommend --profile .\\hardware-profile.json`. Kerna reports the
profile but does not treat it as independent validation or manufacture a
recommendation without a pinned evidence-backed recipe.

## Policy and approvals

Denied tools are omitted from MCP discovery. A tool marked
`require_confirmation` returns a one-time approval request without reaching the
plugin. Approve it locally, then have the client retry the exact call:

```powershell
kerna approval list
kerna approval approve <request-id>
kerna approval reject <request-id>
```

An approval expires after ten minutes and is bound to the workspace, image
digest, tool name, and canonical arguments. Use `kerna status` for containment
and pending-approval state, and `kerna trace <task-id>` for the durable receipt.
