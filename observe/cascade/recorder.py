"""Record what real coding tools actually send, so the design stops being guesswork.

Every routing decision in this package was made against an *imagined* traffic shape. We
have never seen a request from a real coding agent. Three open questions all reduce to
that, and one hour of recorded traffic answers all three:

  1. **Does real traffic stream?** **Answered: yes, every agentic turn does** (6/6, lb
     0.61). The dispatcher refused every streaming request, so that rule excluded all real
     work and the product routed nothing — replaced by `is_agentic` in Decision 036.
  2. **Can we see the gate outcome?** The cascade's whole premise is inheriting the
     customer's existing gate. From the API boundary we return a completion and never
     learn whether the code worked — *unless* the agent feeds its own test output back in
     the next turn, which would hand us the signal for free.
  3. **What task classes exist?** Earned autonomy is per class. A classifier cannot be
     designed against imagination. **Still open** — one client on one task is not a
     distribution.

Questions 1 and 2 are answered, which is why this file has already paid for itself. Its
remaining job is question 3, and a second job it acquired by accident: the traffic it
records is the only place a rule change like 036 can be checked against reality rather
than against the next plausible guess.

## Privacy stance, stated precisely

This is a **local diagnostic**, not the telemetry pipeline. Decision 005's metadata-only
rule governs what leaves the device; this file never leaves it, and the operator is
recording their own traffic on their own machine.

Even so, the default is **structure without content**: roles, counts, sizes, tool names,
flags. Prompt text is captured only with `--record-content`, because a recording that
quietly contains someone's proprietary source code is a liability the moment it is
attached to a bug report — and we would be the ones who made it easy.
"""

from __future__ import annotations

import json
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

SCHEMA = "m0-traffic-v1"
SAMPLE_CHARS = 400


def _classify_blocks(content: Any) -> dict[str, int]:
    """Count the block types in a message body. This is where agent shape shows up."""
    counts: dict[str, int] = {}
    if isinstance(content, str):
        counts["text"] = 1
    elif isinstance(content, list):
        for block in content:
            if isinstance(block, dict):
                kind = str(block.get("type", "unknown"))
                counts[kind] = counts.get(kind, 0) + 1
            elif isinstance(block, str):
                counts["text"] = counts.get("text", 0) + 1
    return counts


# Structural markers only. Recognising a pytest summary or a traceback is reading the
# *shape* of a tool result, not its content -- these strings say nothing about the
# customer's code, and the label is what gets written while the text never does.
_SHAPES: tuple[tuple[str, tuple[str, ...]], ...] = (
    # Test output first: a failing suite contains a traceback, and classifying it as an
    # error would hide the one result shape that carries a correctness signal.
    ("test_output", ("passed", "failed", "test session starts", "assertionerror",
                     "tests ran", "ok (", "FAIL:", "PASS ", "✓", "✗")),
    ("error",       ("traceback (most recent call last)", "error:", "exception",
                     "command not found", "no such file")),
    ("diff",        ("@@ ", "+++ ", "--- ", "<<<<<<<")),
)


def _result_shape(text: str) -> str:
    """A coarse class for a tool result. Names the kind of thing, never the thing.

    This is the field that decides whether a turn had a correctness signal available, so
    it has to distinguish "the agent just ran the tests" from "the agent just read a
    file". Both are `tool_result` blocks of similar size and the existing counters cannot
    tell them apart.
    """
    if not text.strip():
        return "empty"
    low = text.lower()
    for name, markers in _SHAPES:
        if any(m.lower() in low for m in markers):
            return name
    lines = text.splitlines()
    if len(lines) > 3 and sum(1 for ln in lines if ln.startswith((" ", "\t"))) >= 2:
        return "file_content"        # indentation across several lines: source, not prose
    if len(lines) > 2 and all(len(ln) < 120 for ln in lines):
        return "listing"
    return "other"


def _last_action(payload: dict[str, Any]) -> dict[str, Any]:
    """The most recent tool call in the conversation, and the shape of its result.

    **This is the action the model chose on the previous turn**, which is what makes the
    checkable-slice measurement possible without joining across requests: the distribution
    of `last_tool` over a capture *is* the distribution of actions the agent took.

    Both dialects, for the same reason `is_agentic` reads both -- the interceptor defaults
    to the OpenAI endpoint and the traffic we captured was Anthropic, so handling only the
    shape we happened to record would silently report `null` for every turn of the other.
    """
    messages = [m for m in (payload.get("messages") or []) if isinstance(m, dict)]

    name: str | None = None
    call_id: str | None = None
    for message in reversed(messages):                       # most recent first
        content = message.get("content")
        if isinstance(content, list):                        # Anthropic
            for block in reversed(content):
                if isinstance(block, dict) and block.get("type") == "tool_use":
                    name, call_id = block.get("name"), block.get("id")
                    break
        if name is None and message.get("tool_calls"):       # OpenAI
            calls = message["tool_calls"]
            if isinstance(calls, list) and calls and isinstance(calls[-1], dict):
                call = calls[-1]
                name = (call.get("function") or {}).get("name")
                call_id = call.get("id")
        if name is not None:
            break

    if name is None:
        return {"last_tool": None, "last_result_shape": None, "last_result_bytes": 0}

    # The result belonging to that call, matched by id rather than position -- an agent
    # may issue several calls in one turn and they do not come back in order.
    result_text = ""
    for message in reversed(messages):
        if message.get("role") == "tool" and message.get("tool_call_id") == call_id:
            result_text = _text_of(message.get("content"))
            break
        content = message.get("content")
        if isinstance(content, list):
            for block in content:
                if (isinstance(block, dict) and block.get("type") == "tool_result"
                        and block.get("tool_use_id") == call_id):
                    result_text = _text_of(block.get("content"))
                    break
        if result_text:
            break

    return {
        "last_tool": str(name),
        "last_result_shape": _result_shape(result_text) if result_text else "no_result",
        "last_result_bytes": len(result_text),
    }


def _text_of(content: Any) -> str:
    """All the text in a message body, whatever block carries it.

    Counting only `text` blocks was a real measurement bug: in an agent conversation the
    bulk of the payload is **`tool_result` content** (file contents, test output) and
    **`tool_use` input** (the arguments), neither of which has a `text` key. Ignoring them
    made a 21-message request measure the same as a 1-message one, which is exactly
    backwards — the accumulating tool output *is* the cost the cascade exists to reduce.
    """
    if isinstance(content, str):
        return content
    if not isinstance(content, list):
        return ""

    parts: list[str] = []
    for block in content:
        if isinstance(block, str):
            parts.append(block)
        elif isinstance(block, dict):
            if isinstance(block.get("text"), str):
                parts.append(block["text"])
            elif isinstance(block.get("thinking"), str):
                parts.append(block["thinking"])
            if block.get("content") is not None:      # tool_result payload
                parts.append(_text_of(block["content"]))
            if block.get("input") is not None:        # tool_use arguments
                parts.append(json.dumps(block["input"]))
    return "\n".join(parts)


def summarise_request(
    endpoint: str, payload: dict[str, Any], *, with_content: bool = False
) -> dict[str, Any]:
    """Everything we need to answer the three questions, and nothing we do not."""
    messages = payload.get("messages") or []
    tools = payload.get("tools") or []

    roles: dict[str, int] = {}
    blocks: dict[str, int] = {}
    for message in messages:
        if not isinstance(message, dict):
            continue
        role = str(message.get("role", "?"))
        roles[role] = roles.get(role, 0) + 1
        for kind, n in _classify_blocks(message.get("content")).items():
            blocks[kind] = blocks.get(kind, 0) + n

    # The system prompt is the strongest available signal of which tool is calling and
    # what it is doing; its length alone distinguishes an agent from a chat box.
    system = payload.get("system")
    system_chars = len(system) if isinstance(system, str) else len(_text_of(system))

    record: dict[str, Any] = {
        "endpoint": endpoint,
        "model": payload.get("model"),
        "stream": bool(payload.get("stream")),
        "max_tokens": payload.get("max_tokens"),
        "temperature": payload.get("temperature"),
        "n_messages": len(messages),
        "roles": roles,
        "block_types": blocks,
        "n_tools": len(tools),
        # The size of the tool SCHEMAS, not just how many there are.
        #
        # This field is here because its absence caused a real error. We recorded
        # `n_tools: 77` and `total_chars`, read the second as the context size, and
        # concluded agent turns carry 6-8k tokens. The provider was billing 71-85k. The
        # missing ~60k was 77 tool definitions at roughly 790 tokens each -- about 75%
        # of the entire context, invisible to every counter we had.
        #
        # A local model was then configured for a 16k window on that basis and could not
        # fit a single real turn.
        "tool_schema_chars": sum(len(json.dumps(t)) for t in tools if isinstance(t, dict)),
        "tool_names": sorted(
            str(t.get("name")) for t in tools if isinstance(t, dict) and t.get("name")
        )[:40],
        "system_chars": system_chars,
        "total_chars": sum(len(_text_of(m.get("content"))) for m in messages
                           if isinstance(m, dict)),
        # Question 2: does the agent hand us its own test results?
        "has_tool_results": blocks.get("tool_result", 0) > 0,
        "has_tool_use": blocks.get("tool_use", 0) > 0,
    }
    # Which tool the turn is acting on. `tool_names` is the *declared* list -- all 77,
    # alphabetical -- and says nothing about what the agent is doing, which is what the
    # checkable-slice measurement needs (docs/EXPLORE-MVP.md, P1.1).
    record.update(_last_action(payload))

    last_user = next(
        (m for m in reversed(messages)
         if isinstance(m, dict) and m.get("role") == "user"), None
    )
    if last_user is not None:
        text = _text_of(last_user.get("content"))
        record["last_user_chars"] = len(text)
        if with_content:
            record["last_user_sample"] = text[:SAMPLE_CHARS]
    return record


# Anything that could carry a credential. Recorded as a presence flag and a length so a
# difference between two requests is still visible, but the value never reaches the file.
_SECRET_HEADERS = {"authorization", "x-api-key", "proxy-authorization", "cookie"}


def redacted_headers(headers: dict[str, str]) -> dict[str, Any]:
    """Header names and values, with credentials reduced to a length.

    Headers are the only place left where two byte-identical request bodies can still
    differ, so they are the evidence for why every agent turn is sent twice. Recording
    them wholesale would put the customer's key in a diagnostic file, which is exactly
    the liability this recorder exists to avoid.
    """
    out: dict[str, Any] = {}
    for name, value in headers.items():
        key = name.lower()
        if key in _SECRET_HEADERS:
            out[key] = f"<redacted len={len(value)}>"
        elif key in ("content-length", "host", "accept-encoding", "connection"):
            continue     # transport noise, not client intent
        else:
            out[key] = value[:120]
    return out


@dataclass
class TrafficRecorder:
    """Append-only JSONL. Thread-safe because the sidecar serves concurrently."""

    path: Path
    with_content: bool = False
    _lock: threading.Lock = field(default_factory=threading.Lock)
    count: int = 0

    def __post_init__(self) -> None:
        """Mark the start of every run, not just the first.

        The header was written only when the file did not exist, so successive runs
        appended into one undifferentiated stream. A report over that mixture averaged
        pre-fix and post-fix traffic together and produced "57% streaming" -- a number
        describing no session that ever happened. Runs must be separable to be
        comparable.
        """
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self._write({"record": "header", "schema": SCHEMA,
                     "started": time.strftime("%Y-%m-%dT%H:%M:%S"),
                     "with_content": self.with_content})

    def _write(self, row: dict[str, Any]) -> None:
        with self._lock:
            with self.path.open("a", encoding="utf-8") as fh:
                fh.write(json.dumps(row, ensure_ascii=False) + "\n")

    def record(
        self,
        endpoint: str,
        payload: dict[str, Any],
        *,
        route: str,
        reason: str,
        status: int,
        elapsed_ms: float,
        response_meta: dict[str, Any] | None = None,
        headers: dict[str, str] | None = None,
        turn: str | None = None,
    ) -> None:
        try:
            row = {"record": "request", "t": time.time()}
            if turn:
                # The join key. Same value on the gate row and the exploration row for
                # this turn, so three logs can be read as one timeline.
                row["turn"] = turn
            row.update(summarise_request(endpoint, payload, with_content=self.with_content))
            row.update({"route": route, "reason": reason, "status": status,
                        "elapsed_ms": round(elapsed_ms, 1)})
            if response_meta:
                row["response"] = response_meta
            if headers:
                row["headers"] = redacted_headers(headers)
            self._write(row)
            self.count += 1
        except Exception:  # noqa: BLE001
            # A recorder must never be able to break a request. Losing a row is
            # acceptable; losing the customer's completion is not.
            return
