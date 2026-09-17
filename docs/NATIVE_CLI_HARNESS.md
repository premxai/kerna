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
```

The selected provider key comes from that provider's declared environment variable or a hidden
per-request prompt. It is never accepted as a command-line argument. The request contains no tool
schemas, cannot mutate the workspace, and rejects any action-bearing provider response. Questions
and model prose are not written to Kerna's task or evidence database.

This checkpoint uses the trusted host process for provider I/O. It must move provider traffic and
key custody into the trusted broker before any native path receives tools.

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
