# Kerna Guard Protocol Compatibility

WP0 remains in progress. These are the locally available clients, not yet a support promise.

| Client | Version | Intended launch seam | Live gate status |
|---|---:|---|---|
| Claude Code | 2.1.251 | `ANTHROPIC_BASE_URL` to `/anthropic` | Text, deny, bounded allow, and dashboard hold/release exercised |
| Codex CLI | 0.154.0-alpha.6.2 | ignored user config, ephemeral session, custom provider with `wire_api = "responses"` | Text, bounded allow, hold/release, command denial, and patch denial exercised with a deterministic local upstream; provider-backed capture deferred |

The fixtures under `kernel/tests/fixtures` are redacted protocol-development fixtures. The OpenAI
fixture now matches the tool contracts advertised by this Codex build: `exec_command` is a
function tool with a `cmd` argument and `apply_patch` is a custom freeform tool. The deterministic
upstream in `kernel/examples/wp0_openai_fixture_upstream.rs` verified that Codex accepts Kerna's
rewritten terminal response and sends one function result after release. A provider-backed capture
is still required before claiming byte-for-byte validation. No fixture contains provider keys,
prompts, or real repository content.

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

## Codex deterministic-client proof

The local proof used `--ephemeral`, `--ignore-user-config`, `--ignore-rules`, disabled plugins and
apps, and a custom provider with both request and stream retries set to zero. The controlled
upstream could emit only one inert exact command or one marker-file patch.

- Ordinary text reached Codex and terminated normally.
- `exec_command` allow executed `echo KERNA_ALLOW_TEST` once and returned one
  `function_call_output`.
- `exec_command` hold emitted no command event until the local approval was decided, then executed
  `echo KERNA_ASK_TEST` once.
- Denied `exec_command` returned a normal Kerna assistant message after one upstream request.
- Denied `apply_patch` returned a normal Kerna assistant message and did not create the marker file.
- The upstream observed the broker credential, not the agent-plane session credential.

These results prove the installed client/broker compatibility path without using personal Codex
authentication. They do not replace the remaining real OpenAI API capture.
