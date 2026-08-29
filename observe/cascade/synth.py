"""Deliver a completed local answer as the stream the client asked for.

Decision 036 separated two things that had been conflated. Validation needs the whole
output, so a validated answer cannot be *generated* incrementally — but it can be
*delivered* as a stream once complete. `local_can_stream` has gated that obligation and
been False ever since, which meant local routing could never serve anything: **6 of 6
agentic turns stream**, so a serving path that cannot stream serves nothing at all.

This is that half. It takes a finished response and emits the event sequence the client's
protocol requires.

## What it is honest about

This is **not** streaming generation. The tokens were produced before the first byte
leaves, so the client sees the answer arrive at once after a wait, where the cloud would
have trickled it. Time-to-first-token is worse; total time may well be better. Calling it
"streaming" in a sales conversation without that sentence would be a lie of omission.

## Why the gate belongs here rather than after

The response being synthesised came from a model the customer has not agreed to trust,
and this is the moment it becomes an instruction their agent will execute. Governing it
anywhere else means governing it after it has been handed over.

So `local_stream` takes the same `ToolCallGate` the cloud path takes, and the bytes go
through it. One policy, both routes — a locally-served tool call is checked by the same
rules as a cloud one, which is the only arrangement that can be explained to a customer
in a sentence.
"""

from __future__ import annotations

import json
from typing import Any, Callable, Iterator

# Chunk size for text deltas. Small enough that a long answer arrives in several pieces
# rather than one, large enough not to spend the whole budget on SSE framing.
TEXT_CHUNK = 240


def _sse(event: str, data: dict[str, Any]) -> bytes:
    return f"event: {event}\ndata: {json.dumps(data, ensure_ascii=False)}\n\n".encode()


def _chunks(text: str, size: int = TEXT_CHUNK) -> list[str]:
    return [text[i:i + size] for i in range(0, len(text), size)] or [""]


def anthropic_stream(response: dict[str, Any]) -> Iterator[bytes]:
    """A complete Anthropic message as the SSE sequence a client expects.

    The block indices and the `stop_reason` are the parts that must be right: a client
    reading `tool_use` waits for a tool result it will be asked to produce, and a client
    reading `end_turn` does not. Getting that wrong is the hang described in `enforce.py`.
    """
    content = response.get("content") or []
    usage = response.get("usage") or {}

    yield _sse("message_start", {
        "type": "message_start",
        "message": {
            "id": response.get("id") or "msg_local",
            "type": "message",
            "role": "assistant",
            "model": response.get("model") or "local",
            "content": [],
            "stop_reason": None,
            "stop_sequence": None,
            "usage": {
                "input_tokens": usage.get("input_tokens", 0),
                "output_tokens": 0,
            },
        },
    })

    for index, block in enumerate(content):
        if not isinstance(block, dict):
            continue
        kind = block.get("type")

        if kind == "text":
            yield _sse("content_block_start", {
                "type": "content_block_start", "index": index,
                "content_block": {"type": "text", "text": ""},
            })
            for piece in _chunks(str(block.get("text") or "")):
                yield _sse("content_block_delta", {
                    "type": "content_block_delta", "index": index,
                    "delta": {"type": "text_delta", "text": piece},
                })
            yield _sse("content_block_stop",
                       {"type": "content_block_stop", "index": index})

        elif kind == "tool_use":
            # `input` is empty at start and arrives as partial_json, which is the shape
            # every Anthropic client parses. Sending the arguments inline in
            # content_block_start would be simpler and would not match the protocol.
            yield _sse("content_block_start", {
                "type": "content_block_start", "index": index,
                "content_block": {
                    "type": "tool_use",
                    "id": block.get("id") or f"toolu_local_{index}",
                    "name": block.get("name") or "",
                    "input": {},
                },
            })
            payload = json.dumps(block.get("input") or {}, ensure_ascii=False)
            for piece in _chunks(payload):
                yield _sse("content_block_delta", {
                    "type": "content_block_delta", "index": index,
                    "delta": {"type": "input_json_delta", "partial_json": piece},
                })
            yield _sse("content_block_stop",
                       {"type": "content_block_stop", "index": index})

    yield _sse("message_delta", {
        "type": "message_delta",
        "delta": {
            "stop_reason": response.get("stop_reason") or "end_turn",
            "stop_sequence": response.get("stop_sequence"),
        },
        "usage": {"output_tokens": usage.get("output_tokens", 0)},
    })
    yield _sse("message_stop", {"type": "message_stop"})


def openai_stream(response: dict[str, Any]) -> Iterator[bytes]:
    """A complete chat completion as the chunk sequence a client expects."""
    choice = (response.get("choices") or [{}])[0]
    message = choice.get("message") or {}
    base = {
        "id": response.get("id") or "chatcmpl-local",
        "object": "chat.completion.chunk",
        "created": response.get("created") or 0,
        "model": response.get("model") or "local",
    }

    def chunk(delta: dict[str, Any], finish: str | None = None) -> bytes:
        body = dict(base)
        body["choices"] = [{"index": 0, "delta": delta, "finish_reason": finish}]
        return f"data: {json.dumps(body, ensure_ascii=False)}\n\n".encode()

    yield chunk({"role": "assistant"})

    for piece in _chunks(str(message.get("content") or "")):
        if piece:
            yield chunk({"content": piece})

    for position, call in enumerate(message.get("tool_calls") or []):
        if not isinstance(call, dict):
            continue
        fn = call.get("function") or {}
        yield chunk({"tool_calls": [{
            "index": position,
            "id": call.get("id") or f"call_local_{position}",
            "type": "function",
            "function": {"name": fn.get("name") or "",
                         "arguments": fn.get("arguments") or ""},
        }]})

    yield chunk({}, finish=choice.get("finish_reason") or "stop")
    # The terminator is part of the protocol. A client that never sees it waits.
    yield b"data: [DONE]\n\n"


def local_stream(
    response: dict[str, Any],
    *,
    anthropic: bool,
    gate: Any | None = None,
) -> Iterator[bytes]:
    """The served bytes for a local answer, through the same gate the cloud path uses.

    A locally-served tool call is checked by the same rules as a cloud one. Any other
    arrangement is one a customer cannot be told about in a sentence — and this is the
    route where the model is the one they have *not* agreed to trust.
    """
    raw = anthropic_stream(response) if anthropic else openai_stream(response)

    if gate is None:
        yield from raw
        return

    for piece in raw:
        yield from gate.feed(piece)
    yield from gate.finish()
