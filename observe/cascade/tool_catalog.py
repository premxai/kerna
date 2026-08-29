"""Offer the local model its tools as text, because the native field cannot coexist
with a custom grammar.

## The three measurements that force this

    Anthropic endpoint + grammar            -> grammar silently ignored, prose returned
    OpenAI endpoint + grammar + tools       -> HTTP 400
    OpenAI endpoint + grammar + text tools  -> HTTP 200, correct action

llama.cpp builds its own grammar from the `tools` field, and it will not accept a
second one. So a custom grammar and the native tool field are mutually exclusive, and
the only remaining way to constrain the output *and* state the action space is to move
the action space into the prompt.

That makes this a transport, not prompt engineering. It carries exactly the tools that
survived the policy filter, in full, and adds no guidance the native field would not
have carried implicitly.

## The rule it must not break

The local model's action space has to be the *same* action space, or a disagreement
measures the menu rather than the model — the identical error as offering the grammar a
tool the request never carried. So the schemas serialised here are the schemas that
survived `core-v1`, unmodified and unsimplified. A tidied-up schema is a different
offer.

## Why sorted, and why that is safe

Order is fixed by tool name so the same menu produces the same bytes on every run and in
every process; a catalog that varied would make two rows of one cohort incomparable and
would defeat prompt caching for no benefit. `build_tool_call_grammar` already sorts the
same way, which keeps the catalog and the grammar in the same order by construction.

## What it deliberately does not do

It never names the tool the cloud chose. The whole value of the measurement is that the
local model selects without having seen the answer.
"""

from __future__ import annotations

import json
from typing import Any

_HEADER = "AVAILABLE TOOLS"

# States the same obligation the native `tool_choice: required` states, because that
# field is removed along with `tools`. The grammar enforces the shape regardless; this
# is here so the instruction and the constraint do not contradict each other.
_FOOTER = (
    "Choose exactly one available tool that best advances the request.\n"
    "Return only the tool invocation."
)

# v2. The menu gains a way to decline, and the instruction has to offer it explicitly:
# a grammar that permits abstention while the prompt still demands a tool is a
# contradiction, and the model resolves it by guessing.
#
# v1 admitted only real tools, so a model that believed it already knew the answer had no
# legal way to say so. One observed row answered a search request with
# `Write(content=<the answer>, file_path=<invented path>)` — the most answer-shaped tool
# on the menu, pressed into service as a response channel. A benchmark that forbids "no"
# measures button-pressing under duress rather than judgement.
_FOOTER_ABSTAIN = (
    "Choose exactly one available tool that best advances the request.\n"
    # ASCII in the prompt itself. This text is tokenised by whatever local model is
    # loaded, and an unusual glyph costs tokens while adding nothing.
    "If no tool is needed (the request is already answered, or acting would be wrong), "
    "return the tool named __no_tool__ with empty arguments.\n"
    "__no_tool__ is not a real tool and nothing will be executed. Prefer it over "
    "choosing a tool that does not fit.\n"
    "Return only the tool invocation."
)


def _entry(tool: dict[str, Any]) -> tuple[str, str, Any] | None:
    """(name, description, schema) from a tool in either dialect."""
    if not isinstance(tool, dict):
        return None

    if isinstance(tool.get("function"), dict):                    # OpenAI
        fn = tool["function"]
        name = fn.get("name")
        if not name:
            return None
        return str(name), str(fn.get("description") or ""), fn.get("parameters")

    name = tool.get("name")                                       # Anthropic
    if not name:
        return None
    return str(name), str(tool.get("description") or ""), tool.get("input_schema")


def textual_catalog(
    tools: list[dict[str, Any]],
    *,
    allow_abstain: bool = False,
) -> str:
    """The surviving tools as a deterministic instruction block.

    Raises ValueError on an empty menu: a catalog listing no tool, paired with an
    instruction to choose one, is an impossible request. The caller leaves such a turn
    unscorable instead, which is the honest outcome for a request that offered nothing.

    `allow_abstain` states the `__no_tool__` option in the instruction -- the v2
    behaviour. It must be set together with the grammar's own `allow_abstain`: a grammar
    that permits declining while the prompt demands a tool is a contradiction, and a
    prompt offering an option the grammar forbids makes every token illegal.
    """
    entries = []
    for tool in tools or []:
        entry = _entry(tool)
        if entry is not None:
            entries.append(entry)

    if not entries:
        raise ValueError("a textual tool catalog needs at least one tool")

    entries.sort(key=lambda e: e[0])

    blocks = [_HEADER]
    for name, description, schema in entries:
        # sort_keys is determinism, not tidying: JSON object order carries no meaning,
        # so fixing it changes the bytes without changing the offer. The schema itself
        # is passed through exactly as the policy filter left it.
        rendered = json.dumps(
            schema if schema is not None else {"type": "object", "properties": {}},
            sort_keys=True,
            separators=(",", ":"),
        )
        block = f"Tool: {name}"
        if description:
            block += f"\nDescription: {description}"
        block += f"\nArguments schema:\n{rendered}"
        blocks.append(block)

    blocks.append(_FOOTER_ABSTAIN if allow_abstain else _FOOTER)
    return "\n\n".join(blocks)
