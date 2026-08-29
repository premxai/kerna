"""Govern the tools an agent actually uses, at the only seam where they are visible.

Decision 041. `kerna gateway` governs MCP `tools/call` correctly, and the measured session
declared **77 tools of which 2 were MCP** — every action actually taken (`Bash`, `Edit`,
`Glob`, `Read`) was built into the client and never travelled over MCP. A gateway at the
tool protocol would have governed none of them.

A built-in tool call is observable in exactly one place: **the model's response, as a
`tool_use` block, before the client executes it.** That is this seam.

## How a denial has to work

The obvious approach — drop the `tool_use` block — leaves the client waiting forever for a
tool result it will never be asked to produce, because the message still says
`stop_reason: "tool_use"`. So a denial rewrites three things together:

  * the `tool_use` block is replaced by a **text block** saying what was blocked and why,
  * `stop_reason` becomes `end_turn`,
  * and the turn ends cleanly, with the agent free to explain itself or try another way.

The agent is told, in its own transcript, that it was stopped. That is better than a
silent failure and much better than a hang.

## What is held, and for how long

**Text streams through untouched, byte for byte.** Only a `tool_use` block is buffered,
and only from `content_block_start` to `content_block_stop` — typically a few hundred
bytes at the very end of a turn. The tool *name* arrives in `content_block_start`, so a
name-only rule decides instantly and holds nothing.

## Two failure modes, deliberately opposite

Governance is fail-**closed**; the request path is fail-**open**. Conflating them is the
bug this comment exists to prevent.

  * **A policy that cannot be evaluated is a denial.** That is the safe direction and it is
    what fail-closed means.
  * **A parser that breaks is not.** If this module cannot understand the stream, it emits
    everything it was holding, stops gating for the rest of the response, and records that
    it did. A broken enforcer must never break the customer — but it must be *visibly*
    broken rather than silently permissive, which is why the degradation is counted and
    reported rather than swallowed.

## Dialects

Anthropic SSE is gated. **OpenAI streaming is passed through ungated and recorded as
such** — its tool-call deltas carry no block boundary, so gating it properly is a separate
piece of work and pretending otherwise would produce a policy that silently does nothing.
Every byte of traffic measured so far is Anthropic.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from enum import Enum
from typing import Any, Callable, Iterator


class Decision(str, Enum):
    ALLOW = "allow"
    DENY = "deny"


class Mode(str, Enum):
    OBSERVE = "observe"    # rung 1: record actions, decide nothing
    SHADOW = "shadow"      # rung 2: decide and record, change nothing
    ENFORCE = "enforce"    # rung 3: decide and act


@dataclass(frozen=True)
class Rule:
    """One policy rule. `when` inspects the parsed arguments; None means name-only."""

    tool: str                                     # exact name, or "*"
    decision: Decision
    reason: str = ""
    when: Callable[[dict[str, Any]], bool] | None = None

    @property
    def needs_arguments(self) -> bool:
        return self.when is not None

    def matches(self, tool: str) -> bool:
        return self.tool == "*" or self.tool == tool


@dataclass
class Policy:
    """Exact rules beat wildcards; within one specificity level, the first match wins.

    Specified in `docs/POLICY.md` (P1, P2) and executable in `policy-conformance.json`,
    which Kerna's in-loop engine runs too. Two enforcement points are fine; two meanings
    are not, because a customer writes one policy and expects one answer.

    `default` is deliberately ALLOW **at this seam**, and that is the one divergence the
    spec keeps (P5). A fail-closed default is correct for a *configured* policy and
    catastrophic as an *install* default — this engine sits in front of an agent it did
    not configure, so denying every unlisted tool would deny everything on day one, which
    is the onboarding wall named in Decision 040. Kerna's in-loop engine defaults to deny
    for the opposite and equally good reason: the operator configured that runtime.
    """

    rules: tuple[Rule, ...] = ()
    default: Decision = Decision.ALLOW

    def needs_arguments_for(self, tool: str) -> bool:
        return any(r.matches(tool) and r.needs_arguments for r in self.rules)

    @staticmethod
    def decision_for_level(level: str) -> Decision:
        """A shared-vocabulary permission level as a decision at this seam.

        P4: `require_confirmation` means *a human must approve this*. In a streaming
        model response there is no human and no way to ask one, so the action does not
        proceed. Kerna's own runtime already does exactly this when it has no terminal
        (`ApprovalMode::Deny`); this is the same rule at the other enforcement point.

        An unrecognised level denies, per P3 -- a policy that cannot be evaluated is a
        denial, and a typo in an action name is precisely that.
        """
        normalised = str(level).strip().lower()
        if normalised in ("allow", "auto_approve"):
            return Decision.ALLOW
        return Decision.DENY

    def evaluate(self, tool: str, arguments: dict[str, Any] | None) -> tuple[Decision, str]:
        # P1: a rule naming the tool beats a `*` rule wherever each sits in the file;
        # P2: order still decides between rules of equal specificity.
        #
        # This used to be plain first-match-wins, which broke the commonest policy shape
        # there is -- `* -> deny` followed by an allowlist. Kerna's in-loop engine has
        # always let an exact rule win, so one file gave two answers, and the operator
        # had to know which engine read it to predict either. See docs/POLICY.md.
        ordered = [r for r in self.rules if r.tool == tool]
        ordered += [r for r in self.rules if r.tool == "*"]

        for rule in ordered:
            if not rule.matches(tool):
                continue
            if rule.when is None:
                return rule.decision, rule.reason or f"rule on {rule.tool}"
            if arguments is None:
                # Fail closed: a rule we could not evaluate is a denial.
                return Decision.DENY, f"policy on {rule.tool} needs arguments and none arrived"
            try:
                if rule.when(arguments):
                    return rule.decision, rule.reason or f"rule on {rule.tool}"
            except Exception as exc:  # noqa: BLE001
                return Decision.DENY, f"policy predicate failed: {type(exc).__name__}"
        return self.default, "default"


def load_policy(path) -> Policy:
    """A policy from JSON, in the shared vocabulary of docs/POLICY.md.

        {"default": "allow",
         "rules": [{"tool": "Write", "action": "deny", "reason": "read-only pilot"}]}

    `action` uses the same three levels Kerna's in-loop engine uses, so one file can
    describe both enforcement points -- which is the whole reason the conformance suite
    exists. `require_confirmation` resolves to a denial here (P4): there is no human in a
    streaming response to ask.

    A file that cannot be read or parsed raises. Governance is fail-closed on decisions,
    and silently falling back to "allow everything" because a path was mistyped is the
    one failure this module must never have.
    """
    import json
    from pathlib import Path

    body = json.loads(Path(path).read_text(encoding="utf-8"))
    rules = tuple(
        Rule(tool=str(r["tool"]),
             decision=Policy.decision_for_level(r.get("action", "deny")),
             reason=str(r.get("reason", "")))
        for r in body.get("rules", [])
    )
    return Policy(rules=rules,
                  default=Policy.decision_for_level(body.get("default", "allow")))


@dataclass
class GateStats:
    tool_calls: int = 0
    allowed: int = 0
    denied: int = 0
    would_deny: int = 0          # shadow mode: what enforcement would have stopped
    degraded: bool = False       # we stopped gating because we could not parse
    ungated_dialect: bool = False

    def as_dict(self) -> dict[str, Any]:
        return {
            "tool_calls": self.tool_calls, "allowed": self.allowed,
            "denied": self.denied, "would_deny": self.would_deny,
            "degraded": self.degraded, "ungated_dialect": self.ungated_dialect,
        }


def _event(name: str, data: dict[str, Any]) -> bytes:
    return f"event: {name}\ndata: {json.dumps(data)}\n\n".encode()


@dataclass
class ToolCallGate:
    """A streaming filter over an Anthropic SSE response.

    Feed it upstream bytes; it yields bytes to write to the client. Text is forwarded
    immediately and unchanged; only tool blocks are ever held.
    """

    policy: Policy
    mode: Mode = Mode.OBSERVE
    record: Callable[[dict[str, Any]], None] = lambda row: None
    turn: str | None = None
    stats: GateStats = field(default_factory=GateStats)

    _buffer: str = ""                              # incomplete trailing SSE text
    _held: list[bytes] = field(default_factory=list)
    _tool: str | None = None
    _tool_index: int | None = None
    _args: list[str] = field(default_factory=list)
    _denied_this_turn: list[str] = field(default_factory=list)
    # The block a denial replaced. Frames still arriving for it are dropped: the client
    # was told that block ended, and sending more of it contradicts what it already read.
    _suppressed_index: int | None = None
    _passthrough: bool = False

    # ------------------------------------------------------------------ feed

    def feed(self, chunk: bytes | str) -> Iterator[bytes]:
        if self._passthrough:
            yield chunk if isinstance(chunk, bytes) else chunk.encode()
            return
        try:
            yield from self._feed(chunk)
        except Exception as exc:  # noqa: BLE001
            # The plumbing broke. Emit what we were holding, stop gating, keep serving --
            # and say so, because a silently permissive enforcer is worse than none.
            self.stats.degraded = True
            self._safe_record({"record": "gate", "event": "degraded",
                               "error": type(exc).__name__})
            self._passthrough = True
            yield from self._release()

    def _feed(self, chunk: bytes | str) -> Iterator[bytes]:
        text = chunk.decode("utf-8", "replace") if isinstance(chunk, bytes) else chunk
        self._buffer += text

        # SSE frames are separated by a blank line. Anything after the last one is a
        # partial frame and must stay buffered rather than being parsed as truncated JSON.
        while "\n\n" in self._buffer:
            frame, self._buffer = self._buffer.split("\n\n", 1)
            yield from self._frame(frame + "\n\n")

    def _frame(self, frame: str) -> Iterator[bytes]:
        payload = None
        for line in frame.splitlines():
            if line.startswith("data:"):
                blob = line[5:].strip()
                if blob and blob != "[DONE]":
                    try:
                        payload = json.loads(blob)
                    except (ValueError, TypeError):
                        payload = None
                break

        raw = frame.encode()
        if not isinstance(payload, dict):
            yield from self._emit(raw)
            return

        kind = payload.get("type")

        # Anything still arriving for a block we suppressed is dropped. The client was
        # told that block ended; sending more of it contradicts what it already read.
        if (self._suppressed_index is not None
                and kind in ("content_block_delta", "content_block_stop")
                and payload.get("index") == self._suppressed_index):
            if kind == "content_block_stop":
                self._suppressed_index = None
            return

        if payload.get("choices") is not None:      # OpenAI streaming: not gated, said so
            if not self.stats.ungated_dialect:
                self.stats.ungated_dialect = True
                self._safe_record({"record": "gate", "event": "ungated_dialect",
                                   "detail": "openai stream: tool deltas carry no block "
                                             "boundary; forwarded without gating"})
            yield raw
            return

        if kind == "content_block_start":
            block = payload.get("content_block") or {}
            if isinstance(block, dict) and block.get("type") == "tool_use":
                yield from self._open_tool(payload, block, raw)
                return

        elif kind == "content_block_delta" and self._tool is not None:
            if payload.get("index") == self._tool_index:
                delta = payload.get("delta") or {}
                if isinstance(delta, dict) and delta.get("type") == "input_json_delta":
                    self._args.append(str(delta.get("partial_json") or ""))
                self._held.append(raw)
                return

        elif kind == "content_block_stop" and self._tool is not None:
            if payload.get("index") == self._tool_index:
                self._held.append(raw)
                yield from self._close_tool()
                return

        elif kind == "message_delta" and self._denied_this_turn and self.mode is Mode.ENFORCE:
            # A suppressed tool call must not leave `stop_reason: tool_use` behind, or the
            # client waits forever for a result nobody will ask it to produce.
            delta = payload.get("delta") or {}
            if isinstance(delta, dict) and delta.get("stop_reason") == "tool_use":
                fixed = json.loads(json.dumps(payload))
                fixed["delta"]["stop_reason"] = "end_turn"
                yield _event("message_delta", fixed)
                return

        yield from self._emit(raw)

    # ------------------------------------------------------------ tool blocks

    def _open_tool(self, payload: dict, block: dict, raw: bytes) -> Iterator[bytes]:
        self._tool = str(block.get("name") or "")
        self._tool_index = payload.get("index")
        self._args = []
        self.stats.tool_calls += 1

        if self.mode is Mode.OBSERVE:
            self._safe_record({"record": "gate", "event": "observed", "tool": self._tool})
            self._tool = None                        # nothing to hold; stream it
            yield raw
            return

        if not self.policy.needs_arguments_for(self._tool):
            # Decidable on the name alone: no buffering, no added latency.
            decision, reason = self.policy.evaluate(self._tool, {})
            yield from self._apply(decision, reason, [raw])
            return

        self._held = [raw]                           # hold until the arguments complete

    def _close_tool(self) -> Iterator[bytes]:
        tool = self._tool or ""
        try:
            arguments = json.loads("".join(self._args)) if self._args else {}
            if not isinstance(arguments, dict):
                arguments = {}
        except (ValueError, TypeError):
            arguments = None                         # unparsable -> fail closed
        decision, reason = self.policy.evaluate(tool, arguments)
        yield from self._apply(decision, reason, self._held)

    def _apply(self, decision: Decision, reason: str, held: list[bytes]) -> Iterator[bytes]:
        tool, index = self._tool or "", self._tool_index
        self._tool, self._tool_index = None, None
        self._held, self._args = [], []

        row = {"record": "gate", "tool": tool, "decision": decision.value,
               "reason": reason, "mode": self.mode.value}

        if decision is Decision.ALLOW:
            self.stats.allowed += 1
            self._safe_record(row)
            yield from held
            return

        if self.mode is Mode.SHADOW:
            self.stats.would_deny += 1
            row["enforced"] = False
            self._safe_record(row)
            yield from held                          # shadow changes nothing
            return

        self.stats.denied += 1
        row["enforced"] = True
        self._safe_record(row)
        self._denied_this_turn.append(tool)

        # Remember which block was suppressed, so its remaining frames are swallowed too.
        #
        # A name-only rule decides at `content_block_start` and stops tracking the block,
        # so its later `input_json_delta` and `content_block_stop` used to fall through --
        # the client saw a replacement text block, then deltas for a block it had never
        # been told started. Malformed, and invisible until a stream carried the two in
        # separate frames.
        self._suppressed_index = index
        yield from self._denial_text(tool, reason)

    def _denial_text(self, tool: str, reason: str) -> Iterator[bytes]:
        """Replace the suppressed call with a text block the agent can read.

        Told in its own transcript that it was stopped and why, an agent can explain
        itself or try another route. A silently dropped call teaches it nothing.
        """
        index = 99                                   # past any real block index
        message = f"[blocked by policy] {tool} was not permitted: {reason}"
        yield _event("content_block_start",
                     {"type": "content_block_start", "index": index,
                      "content_block": {"type": "text", "text": ""}})
        yield _event("content_block_delta",
                     {"type": "content_block_delta", "index": index,
                      "delta": {"type": "text_delta", "text": message}})
        yield _event("content_block_stop",
                     {"type": "content_block_stop", "index": index})

    # ---------------------------------------------------------------- plumbing

    def _emit(self, raw: bytes) -> Iterator[bytes]:
        if self._tool is not None:
            self._held.append(raw)                   # mid-tool-block: keep ordering
            return
        yield raw

    def _release(self) -> Iterator[bytes]:
        held, self._held = self._held, []
        self._tool, self._tool_index, self._args = None, None, []
        yield from held
        if self._buffer:
            tail, self._buffer = self._buffer, ""
            yield tail.encode()

    def finish(self) -> Iterator[bytes]:
        """Flush anything still held. A truncated stream must not swallow bytes."""
        yield from self._release()

    def _safe_record(self, row: dict[str, Any]) -> None:
        try:
            if self.turn:
                row = {**row, "turn": self.turn}
            self.record(row)
        except Exception:  # noqa: BLE001
            pass                                     # a recorder must never break a stream


# --------------------------------------------------------------- default rules

def starter_policy() -> Policy:
    """A policy worth showing an operator, not one worth shipping as a default.

    Every rule here corresponds to something an agent actually does, and the argument
    predicates exist to make the point that name-only rules are too blunt: `Bash` is not
    dangerous, `rm -rf` is.
    """
    def destructive(args: dict[str, Any]) -> bool:
        cmd = str(args.get("command") or "")
        return any(m in cmd for m in ("rm -rf", "rm -fr", ":(){", "mkfs", "dd if=",
                                      "> /dev/sd", "shutdown", "reboot"))

    def outside_workspace(args: dict[str, Any]) -> bool:
        path = str(args.get("file_path") or args.get("path") or "")
        return path.startswith(("/etc", "/usr", "/sys", "C:\\Windows", "C:\\Program")) \
            or ".." in path

    def credential_file(args: dict[str, Any]) -> bool:
        path = str(args.get("file_path") or args.get("path") or "").lower()
        return any(m in path for m in (".env", "id_rsa", ".aws/credentials",
                                       ".ssh/", "secrets."))

    return Policy(rules=(
        Rule("Bash", Decision.DENY, "destructive shell command", destructive),
        Rule("Edit", Decision.DENY, "writes outside the workspace", outside_workspace),
        Rule("Write", Decision.DENY, "writes outside the workspace", outside_workspace),
        Rule("Read", Decision.DENY, "reads a credential file", credential_file),
    ))
