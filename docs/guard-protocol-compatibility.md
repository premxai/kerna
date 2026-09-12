# Kerna Guard Protocol Compatibility

WP0 remains in progress. These are the locally available clients, not yet a support promise.

| Client | Version | Intended launch seam | Live gate status |
|---|---:|---|---|
| Claude Code | 2.1.251 | `ANTHROPIC_BASE_URL` to `/v1/messages` | Not yet exercised against the Rust broker |
| Codex CLI | 0.154.0-alpha.6.2 | isolated `CODEX_HOME`, custom provider with `wire_api = "responses"` | Not yet exercised against the Rust broker |

The fixtures under `kernel/tests/fixtures` are redacted protocol-development fixtures. They are
synthetic until replaced or validated byte-for-byte against captures from these exact clients.
They contain no provider keys, prompts, or real repository content.

## Current broker routes

`kerna serve` exposes `POST /anthropic/v1/messages` and `POST /openai/v1/responses`.
The agent authenticates only to Kerna; the broker reads `ANTHROPIC_API_KEY` or
`OPENAI_API_KEY` locally and never forwards the agent's bearer token upstream. The temporary
`KERNA_ANTHROPIC_UPSTREAM` and `KERNA_OPENAI_UPSTREAM` environment variables select a test
upstream for redacted fixture validation.

Until the receipt-bound approval service is wired, action calls through these routes deny
fail-closed. Live validation requires each provider key to be made available only to the trusted
broker process; neither key is available in this workspace today.
