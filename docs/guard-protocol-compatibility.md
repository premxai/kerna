# Kerna Guard Protocol Compatibility

WP0 remains in progress. These are the locally available clients, not yet a support promise.

| Client | Version | Intended launch seam | Live gate status |
|---|---:|---|---|
| Claude Code | 2.1.251 | `ANTHROPIC_BASE_URL` to `/v1/messages` | Not yet exercised against the Rust broker |
| Codex CLI | 0.154.0-alpha.6.2 | isolated `CODEX_HOME`, custom provider with `wire_api = "responses"` | Not yet exercised against the Rust broker |

The fixtures under `kernel/tests/fixtures` are redacted protocol-development fixtures. They are
synthetic until replaced or validated byte-for-byte against captures from these exact clients.
They contain no provider keys, prompts, or real repository content.
