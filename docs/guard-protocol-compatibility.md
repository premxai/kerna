# Kerna Guard Protocol Compatibility

WP0 remains in progress. These are the locally available clients, not yet a support promise.

| Client | Version | Intended launch seam | Live gate status |
|---|---:|---|---|
| Claude Code | 2.1.251 | `ANTHROPIC_BASE_URL` to `/anthropic` | Text, deny, bounded allow, and dashboard hold/release exercised |
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

Unknown, malformed, and unsupported action events deny fail-closed. `auto_approve` releases an
action unchanged and `require_confirmation` holds it for local dashboard approval. The browser
shows the protocol, tool name, and SHA-256 digest of canonical arguments; it does not receive raw
arguments from this bridge. The broker expires an unanswered request after five minutes.

This is deliberately a WP0 compatibility bridge, **not** the WP3 approval implementation: its
pending-approval row is not yet bound to a session, agent version, policy digest, worktree
baseline, canonical action digest, and one-time receipt. Do not treat it as a production approval
guarantee. In particular, the dashboard cannot yet show a safe, reviewable command description;
the live smoke test uses only a documented inert `echo` command.

Live validation requires each provider key to be made available only to the trusted broker process;
no provider key is stored in this repository or its fixtures.
