# Kerna Native CLI Harness

Kerna is becoming the terminal-safe front door for model conversations and governed agent work.
The CLI owns provider selection, session lifecycle, tools, policy, approvals, containment, and
evidence. Provider-specific coding agents remain optional headless adapters rather than user-facing
terminal interfaces.

## Product commands

The implementation sequence is intentionally narrow:

1. `kerna ask` — one tool-less question; no prompt or model-prose persistence.
2. `kerna chat` — resumable text sessions with explicit context controls.
3. `kerna code` — contained repository tools, receipts, approvals, diff review, and explicit apply.
4. `--engine claude|codex` — optional certified headless harness adapters behind the same events.

## First checkpoint: `kerna ask`

```text
kerna ask "Explain this error" --provider anthropic
kerna ask "Summarize this design" --provider openai --model <model>
kerna ask "Summarize this design" --provider mock --json
```

The selected provider key comes from that provider's declared environment variable or a hidden
per-request prompt. It is never accepted as a command-line argument. Kerna passes it once over
stdin to a short-lived trusted broker container, then drops the host copy. The request contains no
tool schemas, cannot mutate the workspace, and rejects any action-bearing provider response.
Questions and model prose are not written to Kerna's task or evidence database.

`--json` emits JSON Lines using the stable `session.started`, `assistant.delta`,
`session.completed`, and `session.failed` vocabulary. Anthropic and OpenAI-compatible SSE text is
decoded incrementally and emitted as provider deltas. Human output and JSON output are renderings of
the same internal events. Malformed JSON, action-bearing events, and streams ending mid-event fail
closed.

Provider I/O now leaves only from the broker's dedicated egress network. The CLI connects to a
loopback-only published port using a random session credential; the broker accepts only the exact
Anthropic Messages or OpenAI Chat Completions endpoint and provider domain. The provider key is
held only in broker memory and never enters Docker metadata. Container and network cleanup is
RAII-bound to success, provider failure, malformed streams, and early CLI exit.

This completes the credential and egress boundary for the tool-less command. It does not authorize
tools: adding tools requires canonical actions, containment, policy, approval, and pre-release
receipts as a separate checkpoint.

## Stable internal event direction

Later interactive and automated clients will consume one structured event vocabulary:

```text
session.started
assistant.delta
tool.requested
approval.required
tool.released
tool.result_observed
session.completed | session.failed | session.interrupted
```

The ordinary CLI renders these events as line-oriented text. Browser, desktop, IDE, and CI clients
can consume the same event stream without embedding a third-party full-screen terminal UI.

## Non-negotiable boundary

Skills provide instructions, not authority. Tools provide typed capabilities. Every future tool
request must normalize into `ActionIntent`, pass policy and exact approval when required, execute
inside the declared containment boundary, and commit its receipt before release.
