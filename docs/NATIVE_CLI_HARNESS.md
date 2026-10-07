# Kerna Native CLI Harness

Kerna is becoming the terminal-safe front door for model conversations and governed agent work.
The CLI owns provider selection, session lifecycle, tools, policy, approvals, containment, and
evidence. Provider-specific coding agents remain optional headless adapters rather than user-facing
terminal interfaces.

## Product commands

The implementation sequence is intentionally narrow:

1. `kerna ask` - one tool-less question; no prompt or model-prose persistence.
2. `kerna chat` - in-memory text sessions with explicit context controls.
3. `kerna code` - contained repository tools, receipts, approvals, diff review, and explicit apply.
4. `--engine claude|codex` - optional certified headless harness adapters behind the same events.

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
`context.cleared`, `session.completed`, `session.interrupted`, and `session.failed` vocabulary.
Anthropic and OpenAI-compatible SSE text is decoded incrementally and emitted as provider deltas.
Human output and JSON output are renderings of the same internal events. Malformed JSON,
action-bearing events, and streams ending mid-event fail closed.

Provider I/O now leaves only from the broker's dedicated egress network. The CLI connects to a
loopback-only published port using a random session credential; the broker accepts only the exact
Anthropic Messages or OpenAI Chat Completions endpoint and provider domain. The provider key is
held only in broker memory and never enters Docker metadata. Container and network cleanup is
RAII-bound to success, provider failure, malformed streams, Ctrl+C, and early CLI exit.

This completes the credential and egress boundary for the tool-less command. It does not authorize
tools: adding tools requires canonical actions, containment, policy, approval, and pre-release
receipts as a separate checkpoint.

## Second checkpoint: `kerna chat`

```text
kerna chat --provider anthropic
kerna chat --provider openai --model <model>
kerna chat --provider mock --json
```

`kerna chat` keeps one broker alive for one terminal session and rotates the scoped provider
credential on the next launch. Chat history is held only in process memory. `/clear` discards that
in-memory history and emits `context.cleared`; `/exit` and EOF end the session. There is no default
transcript file, task row, or evidence row containing prompts or model prose.

The chat path still accepts only text turns. Any tool-shaped history, provider tool delta, malformed
SSE, empty answer, provider failure, or Ctrl+C terminates fail-closed. No tools, repository writes,
MCP calls, shell commands, browser mutations, or package/network authority are introduced by this
checkpoint.

## Third checkpoint: `kerna code` dry-run and proposal preflight

```text
kerna code "Plan the parser refactor" --repo . --provider anthropic
kerna code "Plan the parser refactor" --repo . --provider mock --json
```

The first `kerna code` checkpoint is proposal-only. Kerna reads Git metadata from the trusted host:
repository root, HEAD, short status, and a bounded tracked-file list. It does not read file contents,
does not execute repository code, does not apply patches, and does not expose filesystem, process,
MCP, browser, package-manager, or network tools to the model.

The provider still runs through the broker-contained native path. The prompt explicitly tells the
model it is in dry-run mode and must return ordinary prose plus one strict proposal envelope between
`KERNA_PROPOSAL_JSON_BEGIN` and `KERNA_PROPOSAL_JSON_END`. The envelope can describe only
`file_read`, `file_write`, `shell`, `network`, or `package` actions. Unknown fields, missing fields,
unknown action kinds, malformed JSON, missing envelopes, and oversized proposals fail closed.

Kerna normalizes each proposed action through the canonical `ActionIntent` contract and emits a
`proposal.preflight` event containing policy effect, canonical resource, canonical action digest,
required future containment, receipt state, and risk tags. These events are explicitly
non-executable: `executable=false` and `receipt_state=preflight_only_not_requested`. No action is
released, receipted as requested, approved, applied, or executed by this checkpoint.

Prompts, repository metadata, proposals, and model prose are not persisted by default. This
checkpoint creates the product surface for repository work without pretending that model text is an
executable plan. The next checkpoint may add contained inspection only after each action has a
receipt/approval path before release.

## Fourth checkpoint: `kerna code` contained read-only inspection

```text
kerna code "Inspect the README before proposing changes" --repo . --provider anthropic
kerna code "Inspect the README before proposing changes" --repo . --provider mock --json
```

The second `kerna code` checkpoint converts eligible `file_read` proposal actions into
receipt-bound, contained, read-only inspection. Every other action kind — `file_write`, `shell`,
`network`, `package` — stays preflight-only and is never executed.

Each eligible `file_read` becomes a guard action binding (session, agent version, policy digest,
worktree baseline digest, canonical action digest) whose `requested` plus `released` receipt rows
commit atomically before any read happens. The read result is then recorded as `result_observed`
with digest-only details: bytes read, content SHA-256, and a truncation flag. If the read or the
observation fails after release, the receipt is marked `outcome_unknown` and never reported as
executed. File content is never persisted to the database; only the bounded preview returned to the
caller in the event stream carries content.

Inspection is contained to the trusted CLI process and the resolved repository worktree boundary —
the containment label is `trusted_cli_worktree_read`, and container containment is not claimed or
credited for this path. Targets fail closed on absolute paths, Windows drive/UNC/verbatim forms,
home references, `..` traversal, reserved device names, `.git` internals, canonical secret paths,
the evidence database and its sidecar files, symlink escapes outside the worktree (resolved
post-canonicalization), missing or unresolvable paths, non-regular files, and files above the
512 KiB cap. Reads return at most an 8 KiB preview with a full-content SHA-256 digest. Policy `deny`
blocks the read; policy `ask` stays preflight-only because this checkpoint has no approval surface;
unreceiptable actions fail closed.

The command remains a dry-run for everything except these contained reads: no writes, shell
execution, package installs, network fetches, patch apply, or original repository mutation of any
kind.

## Stable internal event direction

Later interactive and automated clients will consume one structured event vocabulary:

```text
session.started
assistant.delta
proposal.preflight
inspection.requested | inspection.blocked
inspection.released
inspection.result_observed | inspection.outcome_unknown
context.cleared
tool.requested
approval.required
tool.released
tool.result_observed
session.completed | session.failed | session.interrupted
```

The `inspection.*` events are the read-only specialization of the future `tool.*` vocabulary: the
same requested/released/result-observed lifecycle with an explicit fail-closed blocked state and an
honest outcome-unknown state. The ordinary CLI renders these events as line-oriented text. Browser,
desktop, IDE, and CI clients can consume the same event stream without embedding a third-party
full-screen terminal UI.

## Non-negotiable boundary

Skills provide instructions, not authority. Tools provide typed capabilities. Every future tool
request must normalize into `ActionIntent`, pass policy and exact approval when required, execute
inside the declared containment boundary, and commit its receipt before release.
