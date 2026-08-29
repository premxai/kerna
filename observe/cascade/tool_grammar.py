"""Force a tool call, because asking for one did not work.

The Legion runs established the need precisely. Qwen2.5-Coder-7B, given real Claude Code
contexts that fitted comfortably in a 32K window with a reduced tool menu, answered in
**prose**: 1,323 characters, `wrapper: none`, no malformed call for a parser to rescue.
It did so even under `tool_choice: required`, which llama.cpp evidently treats as a hint —
generation ran to `stop: max_tokens` and produced 2,486 characters of text.

Ten of ten turns came back `unscorable`. Not disagreement: **no action was emitted at
all**, so there was nothing to compare.

A grammar is not a hint. Under this one, prose is unrepresentable.

## Why this is legitimate rather than a thumb on the scale

The product controls the local request completely. It never needs a local model to
*spontaneously* behave like an agent — it needs the model to produce the same action the
cloud produced. Forcing the output shape is exactly what Decision 024 adopted constrained
decoding for, and that decision carries a measured result: 9 fabricated identifiers per
100 generations, all prevented.

The comparison stays honest because **the choice of tool and of arguments remains entirely
the model's.** Only the serialisation is compelled.

## What it deliberately does not constrain

Arguments are free-form JSON. Constraining them would be constraining the *answer* rather
than its shape, and a grammar that dictates the answer measures the grammar. The tool name
is a closed set and is constrained, which also makes `action_space_ineligible` impossible
by construction on the local side — the model cannot name a tool it was not offered.

## Cohort discipline

A rate earned under a grammar is a different measurement from one earned without it, and
pooling them would repeat exactly the error `class_version` exists to prevent. Runs carry
`decoding=grammar-v1`.

One consequence to state plainly, because it is the honest cost: a model compelled to emit
a tool call will emit one even when answering in prose was the better judgement. That shows
up as a *wrong action* rather than as silence — which is a worse-looking number and a far
more useful one, because `different_action` is evidence and `unscorable` is not.
"""

from __future__ import annotations

import json
from typing import Iterable

# Kept local rather than imported from the eval harness's grammar module: that one serves the corpora and
# its grammars are checked against gbnf.py's parser. This one is for the live EXPLORE path
# and answers to llama.cpp's sampler, so the two are deliberately not coupled.
_JSON_TAIL = """
object ::= "{" ws ( pair ( ws "," ws pair )* )? ws "}"
pair ::= string ws ":" ws value
value ::= string | number | object | array | "true" | "false" | "null"
array ::= "[" ws ( value ( ws "," ws value )* )? ws "]"
string ::= "\\"" char* "\\""
char ::= [^"\\\\] | "\\\\" ["\\\\/bfnrtu]
number ::= "-"? [0-9]+ ( "." [0-9]+ )? ( [eE] [-+]? [0-9]+ )?
ws ::= [ \\t\\n]*
"""


def gbnf_escape(text: str) -> str:
    """Escape a literal for a GBNF double-quoted string."""
    return text.replace("\\", "\\\\").replace('"', '\\"')


# The sentinel a model uses to decline. Not a tool: it is never offered to the customer's
# agent, never forwarded, and never executed.
#
# v1 admitted only real tool names, so a model that believed it already knew the answer
# had no legal way to say so and had to pick something. One observed row answered a search
# request with `Write(content=<the answer>, file_path=<invented path>)` -- the most
# answer-shaped tool on the menu, pressed into service as a response channel.
#
# That row is a genuine safety finding *and* partly our artifact, which is exactly why the
# option has to exist: a benchmark that forbids "no" measures button-pressing under
# duress, not judgement.
NO_TOOL = "__no_tool__"


def build_tool_call_grammar(
    tool_names: Iterable[str],
    *,
    allow_abstain: bool = False,
) -> str:
    """GBNF admitting exactly one tool call, with the name drawn from a closed menu.

    Emits `{"name": <one of the offered tools>, "arguments": {...}}` and nothing else.

    `allow_abstain` adds the `__no_tool__` sentinel to the menu -- the v2 behaviour.
    Default off, so v1 grammars are byte-identical to the ones that produced the v1
    cohorts and those runs stay reproducible.
    """
    names = sorted({str(n).strip() for n in tool_names if str(n).strip()})
    if not names:
        raise ValueError("a tool-call grammar needs at least one tool")

    if allow_abstain:
        # Appended after the check above on purpose: abstention is an option alongside
        # real tools, never a menu of its own. A request offering nothing but "decline"
        # would produce a guaranteed refusal and call it a measurement.
        names = sorted(set(names) | {NO_TOOL})

    # Two escapes, in this order, and the order is the point. The model must emit a
    # *JSON* string, so a name containing a quote has to reach it as \" -- then that
    # whole JSON literal is placed inside a GBNF double-quoted literal, which needs its
    # own escaping. Doing only the second produced a grammar demanding invalid JSON.
    # Tool names do not contain quotes today; grammars that are only correct for the
    # inputs you happened to try are how a sampler starts rejecting every token.
    alternatives = " | ".join(
        f'"{gbnf_escape(json.dumps(name))}"' for name in names)
    root = (
        'root ::= "{" ws "\\"name\\"" ws ":" ws name ws "," ws '
        '"\\"arguments\\"" ws ":" ws object ws "}"'
    )
    return f"{root}\nname ::= {alternatives}\n{_JSON_TAIL.lstrip()}"


def offered_tools(payload: dict) -> list[str]:
    """Tool names in a request, in either dialect. The menu the grammar is built from."""
    names: list[str] = []
    for tool in payload.get("tools") or []:
        if not isinstance(tool, dict):
            continue
        if tool.get("name"):                                  # Anthropic
            names.append(str(tool["name"]))
        elif isinstance(tool.get("function"), dict):          # OpenAI
            fn_name = tool["function"].get("name")
            if fn_name:
                names.append(str(fn_name))
    return names
