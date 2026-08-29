"""Translate an Anthropic request into an OpenAI chat request — local shadow only.

## Why this exists

Measured on the Legion, decisively: **llama.cpp's `/v1/messages` endpoint ignores the
`grammar` field.** A grammar admitting exactly one string, `root ::= "GRAMMAR_OK"`,
returned ordinary prose about the ocean. The identical request sent to
`/v1/chat/completions` returned exactly `GRAMMAR_OK`.

So the whole `grammar-v1` cohort was a no-op wearing a cohort label — the most dangerous
kind of result, because every row said `local_decoding=grammar-v1` and none of them had
been constrained by anything. The previous grammar run is void, not disappointing.

The fix is not to abandon the Anthropic dialect. Claude Code speaks it, the cloud request
must keep speaking it, and the customer's traffic is never touched. Only the **local
shadow** is translated, and only when a grammar is in force.

## Why the translation is a measurement risk, and what is done about it

The comparison is local-vs-cloud on the same turn. If the translation quietly drops part
of the prompt, the local model answers a different question, and the disagreement measured
is *ours* rather than the model's — the identical failure mode as a self-authored corpus
(023) and as the tool schemas that were 75% of the context while nothing recorded them
(042).

So this module never drops anything silently. Every block kind it cannot represent is
returned in `dropped`, recorded on the explore row, and a turn that dropped anything is
not a clean observation. An absence that is visible can be excluded; an absence that is
invisible corrupts the rate.
"""

from __future__ import annotations

import json
from typing import Any

# Sampling and control fields that carry the same meaning under both dialects. `grammar`
# is included deliberately: it is the entire reason this translation exists.
_PASSTHROUGH: tuple[str, ...] = (
    "temperature",
    "top_p",
    "top_k",
    "max_tokens",
    "grammar",
)


def _system_text(system: Any) -> str | None:
    """Anthropic's top-level `system`, as a single string."""
    if isinstance(system, str):
        return system or None
    if isinstance(system, list):
        parts = [
            block["text"]
            for block in system
            if isinstance(block, dict)
            and block.get("type") == "text"
            and isinstance(block.get("text"), str)
        ]
        if parts:
            return "\n\n".join(parts)
    return None


def _result_text(content: Any, dropped: set[str]) -> str:
    """Flatten a tool_result's payload to text, recording what could not be flattened."""
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        parts: list[str] = []
        for block in content:
            if not isinstance(block, dict):
                dropped.add("tool_result:non_block")
                continue
            if block.get("type") == "text" and isinstance(block.get("text"), str):
                parts.append(block["text"])
            else:
                dropped.add(f"tool_result:{block.get('type') or 'unknown'}")
        return "\n".join(parts)
    if content is None:
        return ""
    dropped.add("tool_result:non_text")
    return ""


def _translate_tools(tools: Any) -> list[dict[str, Any]]:
    """Anthropic tool declarations to OpenAI function declarations.

    Currently unexercised by any shippable cohort, and deliberately kept. llama.cpp
    returns HTTP 400 for a custom grammar sent alongside `tools`, so every configuration
    that reaches this translation has already moved its menu into the prompt
    (`tool_catalog`) and removed the field. This path is here for the day that
    restriction lifts or another local server is used — and it stays tested rather than
    rotting quietly, because an untested translation is how the local model ends up
    seeing a different action space than the cloud.
    """
    out: list[dict[str, Any]] = []
    for tool in tools or []:
        if not isinstance(tool, dict):
            continue
        if isinstance(tool.get("function"), dict):      # already OpenAI
            out.append(tool)
            continue
        name = tool.get("name")
        if not name:
            continue
        fn: dict[str, Any] = {
            "name": name,
            # An absent schema must still be an object schema. llama.cpp will reject a
            # function whose parameters are null, and that rejection would arrive as a
            # 400 blamed on the grammar.
            "parameters": tool.get("input_schema")
            or {"type": "object", "properties": {}},
        }
        if isinstance(tool.get("description"), str):
            fn["description"] = tool["description"]
        out.append({"type": "function", "function": fn})
    return out


def _translate_tool_choice(choice: Any) -> Any:
    """Anthropic tool_choice to its OpenAI equivalent.

    `any` and `required` mean the same thing — emit some tool call — and the local
    cohort under test sets exactly that. Losing it here would silently reopen the door
    to prose that the grammar is meant to have closed.
    """
    if isinstance(choice, str):                          # already OpenAI
        return choice
    if not isinstance(choice, dict):
        return None
    kind = choice.get("type")
    if kind == "any":
        return "required"
    if kind == "auto":
        return "auto"
    if kind == "none":
        return "none"
    if kind == "tool" and choice.get("name"):
        return {"type": "function", "function": {"name": choice["name"]}}
    if kind == "function" and isinstance(choice.get("function"), dict):
        return choice
    return None


def to_openai_chat(body: dict[str, Any]) -> tuple[dict[str, Any], list[str]]:
    """Return an OpenAI chat body and the sorted list of block kinds dropped.

    A non-empty `dropped` means the local model was shown less than the cloud was, and
    the turn is not a clean comparison.
    """
    dropped: set[str] = set()
    messages: list[dict[str, Any]] = []

    system = _system_text(body.get("system"))
    if system:
        messages.append({"role": "system", "content": system})

    for message in body.get("messages") or []:
        if not isinstance(message, dict):
            dropped.add("message:non_object")
            continue

        role = message.get("role")
        content = message.get("content")

        if isinstance(content, str):
            messages.append({"role": role, "content": content})
            continue

        if not isinstance(content, list):
            dropped.add(f"content:{type(content).__name__}")
            continue

        texts: list[str] = []
        tool_calls: list[dict[str, Any]] = []
        tool_messages: list[dict[str, Any]] = []

        for block in content:
            if not isinstance(block, dict):
                dropped.add("block:non_object")
                continue

            kind = block.get("type")

            if kind == "text" and isinstance(block.get("text"), str):
                texts.append(block["text"])

            elif kind == "tool_use":
                tool_calls.append(
                    {
                        "id": block.get("id") or "",
                        "type": "function",
                        "function": {
                            "name": block.get("name") or "",
                            # OpenAI carries arguments as a JSON *string*, Anthropic as
                            # an object. json.dumps is the translation, not cosmetics.
                            "arguments": json.dumps(block.get("input") or {}),
                        },
                    }
                )

            elif kind == "tool_result":
                tool_messages.append(
                    {
                        "role": "tool",
                        "tool_call_id": block.get("tool_use_id") or "",
                        "content": _result_text(block.get("content"), dropped),
                    }
                )

            else:
                # Thinking blocks, images, documents, and anything a future API version
                # adds. Named, never silently discarded.
                dropped.add(f"block:{kind or 'unknown'}")

        # Ordering is a protocol requirement, not a preference: every `tool` message
        # must directly follow the assistant turn whose call it answers. Anthropic
        # carries those results inside the *next user* message, so they are emitted
        # first, and any accompanying user prose follows as its own message.
        messages.extend(tool_messages)

        if role == "assistant" and tool_calls:
            out: dict[str, Any] = {
                "role": "assistant",
                "content": "\n".join(texts) if texts else None,
                "tool_calls": tool_calls,
            }
            messages.append(out)
        elif texts:
            messages.append({"role": role, "content": "\n".join(texts)})
        elif not tool_messages and not tool_calls:
            dropped.add("message:empty_after_translation")

    out_body: dict[str, Any] = {"messages": messages}

    if body.get("tools"):
        translated = _translate_tools(body["tools"])
        if translated:
            out_body["tools"] = translated

    choice = _translate_tool_choice(body.get("tool_choice"))
    if choice is not None and "tools" in out_body:
        out_body["tool_choice"] = choice

    for key in _PASSTHROUGH:
        if key in body:
            out_body[key] = body[key]

    if body.get("stop_sequences"):
        out_body["stop"] = body["stop_sequences"]

    return out_body, sorted(dropped)
