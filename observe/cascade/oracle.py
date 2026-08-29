"""Did the local model choose the same action as the cloud? (P1.2-C)

Decision 033's surviving insight: every forwarded request returns a cloud answer, and
that answer is **free ground truth we are already paying for.** This module is what turns
it into evidence.

## What is compared, and why it is not text

An agentic turn's output is a tool call — edit this file, run this command, read that
path. So two answers agree when they propose the same action, and comparing the prose
around it would measure writing style. `Action` is the whole comparable surface: a tool,
a target, and the arguments.

Four verdicts, kept separate because merging them destroys the finding:

  * `same_action_same_args` — the same edit to the same file
  * `same_action` — the same file, different content: much closer than a different file,
    and not the same answer
  * `different_action` — a different tool or a different target
  * `unscorable` — one side produced prose, or no action at all

## The warning that belongs on every number this produces

**The cloud is not ground truth.** It is a strong baseline that ships a wrong answer past
a test gate 3.8% of the time by our own measurement. Agreement with it is a *proxy* for
correctness, which is exactly why Decision 034 specifies ε-audits, and an agreement rate
must never be reported as an accuracy. Two models can agree and both be wrong; more
often here, they can disagree and both be right, because there is usually more than one
correct edit.

Argument comparison is **string equality after normalisation** — no semantic analysis,
no AST diffing. That is a deliberately strict bar, and it means `same_action_same_args`
is a **lower bound on agreement** rather than a measurement of it. Reporting it as
anything else would understate the local model in a way that flatters our own thesis.

## Accumulation is a filter, not a buffer

Comparing against a streamed cloud answer means reassembling it from SSE, and this is the
first component in the product that holds a customer's model output at all — the largest
new privacy surface in the design, named as such in the build plan.

So it does not buffer the response. It **extracts only the tool call** as events arrive
and discards everything else, which means the prose the model wrote is never held, never
counted and never available to leak. Combined with a hard byte cap, the exposure is a
tool name, a file path and a JSON argument blob, for as long as one comparison takes.
"""

from __future__ import annotations

import json
import re
import xml.etree.ElementTree as ET
from dataclasses import dataclass
from enum import Enum
from typing import Any

from .tool_grammar import NO_TOOL

# A tool call's arguments are small. Anything past this is a runaway, not an edit, and
# the cap exists so a pathological response cannot grow the process on a laptop.
MAX_ARGS_BYTES = 256 * 1024

# Argument names that identify *what is being acted on*, most specific first. Two edits
# to the same path are a near-miss; two edits to different files are a real disagreement,
# and without a target every comparison collapses to "both called Edit".
_TARGET_KEYS: tuple[str, ...] = (
    "file_path", "notebook_path", "path", "filename", "file",
    "command", "pattern", "url", "query",
)


class Agreement(str, Enum):
    SAME_ACTION_SAME_ARGS = "same_action_same_args"
    SAME_ACTION = "same_action"
    DIFFERENT_ACTION = "different_action"
    UNSCORABLE = "unscorable"


@dataclass(frozen=True)
class Action:
    """One proposed tool call, reduced to the part worth comparing."""

    tool: str
    target: str | None
    args: dict[str, Any]
    truncated: bool = False

    def normalised_args(self) -> str:
        """Sorted keys, collapsed whitespace in string values, stable separators.

        Not semantic equivalence. Two edits that differ only in indentation compare as
        different, which is why the strict verdict is a lower bound.
        """
        def clean(value: Any) -> Any:
            if isinstance(value, str):
                return " ".join(value.split())
            if isinstance(value, dict):
                return {k: clean(v) for k, v in sorted(value.items())}
            if isinstance(value, list):
                return [clean(v) for v in value]
            return value

        return json.dumps(clean(self.args), sort_keys=True, separators=(",", ":"))


def _target_of(args: dict[str, Any]) -> str | None:
    for key in _TARGET_KEYS:
        value = args.get(key)
        if isinstance(value, str) and value.strip():
            return value.strip()
    return None


def _action(tool: Any, args: Any, *, truncated: bool = False) -> Action | None:
    if not isinstance(tool, str) or not tool:
        return None
    if isinstance(args, str):
        try:
            args = json.loads(args)
        except (ValueError, TypeError):
            args = {"_raw": args}
    if not isinstance(args, dict):
        args = {}
    return Action(tool=tool, target=_target_of(args), args=args, truncated=truncated)


def extract_action(response: dict[str, Any] | None) -> Action | None:
    """The tool call in a completed response, in either dialect. None means prose."""
    if not isinstance(response, dict):
        return None

    for block in response.get("content") or []:                       # Anthropic
        if isinstance(block, dict) and block.get("type") == "tool_use":
            return _action(block.get("name"), block.get("input"))

        # Qwen2.5-Coder may emit a valid tool decision as textual XML even when
        # llama.cpp's Anthropic adapter does not promote it to a tool_use block.
        # Treat only the narrow function-call shape as an action; ordinary prose
        # remains unscorable.
        if isinstance(block, dict) and block.get("type") == "text":
            text = block.get("text")
            if isinstance(text, str) and "<function" in text:
                try:
                    import xml.etree.ElementTree as ET

                    root = ET.fromstring(text.strip())
                    fn = root.find(".//function")
                    if fn is not None:
                        name = fn.attrib.get("name")
                        arguments = fn.attrib.get("arguments", "{}")
                        action = _action(name, arguments)
                        if action is not None:
                            return action
                except Exception:  # noqa: BLE001
                    pass

    for choice in response.get("choices") or []:                      # OpenAI
        if not isinstance(choice, dict):
            continue
        message = choice.get("message")
        if not isinstance(message, dict):
            continue
        calls = message.get("tool_calls")
        if isinstance(calls, list) and calls and isinstance(calls[0], dict):
            fn = calls[0].get("function") or {}
            return _action(fn.get("name"), fn.get("arguments"))
    return None



_FUNCTION_CALL_JSON = re.compile(
    r"<function_call>\s*(\{.*\})\s*</function_call>",
    re.IGNORECASE | re.DOTALL,
)

_FUNCTION_CALL_CAMEL = re.compile(
    r"<functionCall\b[^>]*>.*?</functionCall>",
    re.DOTALL,
)


_TOOLS_JSON = re.compile(
    r"<tools>\s*(\{.*?\})\s*</tools>",
    re.IGNORECASE | re.DOTALL,
)


def _response_text_blocks(response: dict[str, Any] | None) -> list[str]:
    """Return assistant text blocks without interpreting arbitrary prose."""
    if not isinstance(response, dict):
        return []

    out: list[str] = []

    # Anthropic
    for block in response.get("content") or []:
        if (
            isinstance(block, dict)
            and block.get("type") == "text"
            and isinstance(block.get("text"), str)
        ):
            out.append(block["text"])

    # OpenAI
    for choice in response.get("choices") or []:
        if not isinstance(choice, dict):
            continue
        message = choice.get("message")
        if not isinstance(message, dict):
            continue
        content = message.get("content")
        if isinstance(content, str):
            out.append(content)

    return out


def _allowed_text_action(
    name: Any,
    args: Any,
    allowed_tools: frozenset[str],
) -> Action | None:
    """Construct an action only when the model was actually offered that tool."""
    if not isinstance(name, str) or name not in allowed_tools:
        return None

    if isinstance(args, str):
        try:
            args = json.loads(args)
        except (ValueError, TypeError):
            return None

    if not isinstance(args, dict):
        return None

    return _action(name, args)


def _snake_text_call(
    text: str,
    allowed_tools: frozenset[str],
) -> Action | None:
    """Parse the exact <function_call>{JSON}</function_call> form we observed."""
    matches = list(_FUNCTION_CALL_JSON.finditer(text))

    # Multiple textual calls are ambiguous. Do not choose one.
    if len(matches) != 1:
        return None

    try:
        call = json.loads(matches[0].group(1))
    except (ValueError, TypeError):
        return None

    if not isinstance(call, dict):
        return None

    return _allowed_text_action(
        call.get("name"),
        call.get("arguments"),
        allowed_tools,
    )


def _camel_text_call(
    text: str,
    allowed_tools: frozenset[str],
) -> Action | None:
    """Parse the exact XML <functionCall> form we observed from /v1/messages.

    Only flat argument elements are accepted. If Qwen emits ambiguous or nested XML,
    leave the result unscorable rather than guessing how to translate it.
    """
    matches = list(_FUNCTION_CALL_CAMEL.finditer(text))

    if len(matches) != 1:
        return None

    try:
        root = ET.fromstring(matches[0].group(0))
    except ET.ParseError:
        return None

    name_node = root.find("name")
    args_node = root.find("arguments")

    if (
        name_node is None
        or args_node is None
        or not isinstance(name_node.text, str)
    ):
        return None

    args: dict[str, Any] = {}

    for child in list(args_node):
        # Nested argument structures are intentionally not inferred.
        if list(child):
            return None

        # Duplicate keys are ambiguous.
        if child.tag in args:
            return None

        args[child.tag] = child.text or ""

    return _allowed_text_action(
        name_node.text.strip(),
        args,
        allowed_tools,
    )



def local_response_diagnostics(
    response: dict[str, Any] | None,
) -> dict[str, Any]:
    """Describe local response shape without retaining model prose."""
    diag: dict[str, Any] = {
        "local_response_dialect": "unknown",
        "local_stop_reason": None,
        "local_text_present": False,
        "local_text_chars": 0,
        "local_structured_tool_call": False,
        "local_text_wrapper": "none",
        "local_text_wrapper_count": 0,
    }

    if not isinstance(response, dict):
        return diag

    # Anthropic response shape.
    if isinstance(response.get("content"), list):
        diag["local_response_dialect"] = "anthropic"
        diag["local_stop_reason"] = response.get("stop_reason")

        for block in response["content"]:
            if not isinstance(block, dict):
                continue
            if block.get("type") == "tool_use":
                diag["local_structured_tool_call"] = True

    # OpenAI response shape.
    elif isinstance(response.get("choices"), list):
        diag["local_response_dialect"] = "openai"

        choices = response["choices"]
        if choices and isinstance(choices[0], dict):
            diag["local_stop_reason"] = choices[0].get("finish_reason")

            message = choices[0].get("message")
            if isinstance(message, dict) and message.get("tool_calls"):
                diag["local_structured_tool_call"] = True

    texts = _response_text_blocks(response)

    diag["local_text_present"] = bool(texts)
    diag["local_text_chars"] = sum(len(text) for text in texts)

    wrappers: list[str] = []

    for text in texts:
        wrappers.extend(
            ["function_call"] * text.lower().count("<function_call")
        )
        wrappers.extend(
            ["functionCall"] * text.count("<functionCall")
        )
        wrappers.extend(
            ["tool_call"] * text.lower().count("<tool_call")
        )
        wrappers.extend(
            ["tools"] * len(_TOOLS_JSON.findall(text))
        )

    diag["local_text_wrapper_count"] = len(wrappers)

    kinds = set(wrappers)
    if len(kinds) == 1:
        diag["local_text_wrapper"] = next(iter(kinds))
    elif len(kinds) > 1:
        diag["local_text_wrapper"] = "multiple"

    return diag



def _tools_text_call(
    text: str,
    allowed_tools: frozenset[str],
) -> Action | None:
    """Parse the exact <tools>{JSON}</tools> form observed under required mode.

    This compatibility form is intentionally not enabled for natural/auto behavior.
    More than one complete tool block is ambiguous and remains unscorable.
    """
    matches = list(_TOOLS_JSON.finditer(text))

    if len(matches) != 1:
        return None

    try:
        call = json.loads(matches[0].group(1))
    except (ValueError, TypeError):
        return None

    if not isinstance(call, dict):
        return None

    return _allowed_text_action(
        call.get("name"),
        call.get("arguments"),
        allowed_tools,
    )


# Returned instead of an Action when the model used the v2 sentinel to decline. A
# distinct object rather than None, because "chose not to act" and "produced nothing we
# could read" are different findings and collapsing them loses the whole point of v2.
ABSTAINED = "abstained"


def _grammar_json_action(
    text: str,
    allowed_tools: frozenset[str],
    *,
    allow_abstain: bool = False,
) -> Action | str | None:
    """The exact shape `build_tool_call_grammar` compels, and nothing else.

    The grammar emits a bare `{"name": ..., "arguments": {...}}` object with no wrapper,
    which every other extractor here refuses on purpose — bare JSON is indistinguishable
    from a model quoting a config file, and accepting it generally would manufacture
    actions out of prose.

    What makes it safe here is that acceptance is gated on the grammar cohort, where the
    sampler could not have produced anything else. Each condition below is a check that
    the grammar was actually in force:

    * the **whole** text parses as one object — a JSON blob sitting inside prose means
      it did not come from this grammar, because prose was unrepresentable;
    * the key set is exactly `name` and `arguments` — the grammar's root admits no
      other key, so an extra one is proof the constraint was not applied;
    * the name is one the request offered — the grammar's menu is closed;
    * `arguments` is an object, as the root's `object` production requires.

    Anything failing these is left unscorable, which is the honest verdict for output
    whose provenance cannot be established.
    """
    stripped = text.strip()
    if not stripped.startswith("{"):
        return None

    try:
        obj = json.loads(stripped)
    except (ValueError, TypeError):
        return None

    if not isinstance(obj, dict) or set(obj) != {"name", "arguments"}:
        return None

    name = obj["name"]
    args = obj["arguments"]

    if not isinstance(name, str):
        return None

    if name == NO_TOOL:
        # Admitted only for the cohort whose grammar offered it. Outside v2 the sentinel
        # is not in the menu, so seeing it means the constraint was not in force and the
        # turn should stay unscorable rather than be read as a decision.
        return ABSTAINED if allow_abstain else None

    if name not in allowed_tools:
        return None
    if not isinstance(args, dict):
        return None

    return _action(name, args)


def extract_local_action(
    response: dict[str, Any] | None,
    *,
    allowed_tools: frozenset[str],
    allow_tools_wrapper: bool = False,
    allow_grammar_json: bool = False,
    allow_abstain: bool = False,
) -> tuple[Action | str | None, str]:
    """Extract a local EXPLORE action and identify its wire format.

    Structured provider output is always preferred.

    The textual fallback exists only because the measured Qwen2.5-Coder/llama.cpp
    combination has explicitly emitted function calls as text despite being given a
    tool-capable chat template. Arbitrary JSON or prose is never treated as an action.

    `allow_grammar_json` admits the bare object a forced grammar produces, and is set
    only for that cohort. It is deliberately not a general JSON parser: outside the
    grammar there is no way to tell an intended action from a quoted example.

    Returns:
        (action, "native")
        (action, "text_function_call_v1")
        (action, "grammar_json_v1")
        (None, "none")
    """
    native = extract_action(response)

    if native is not None:
        return native, "native"

    if not allowed_tools:
        return None, "none"

    recovered: list[tuple[Action, str]] = []

    for text in _response_text_blocks(response):
        action = None
        fmt = "text_function_call_v1"

        # Tried first, because it is the strictest: it requires the entire block to be
        # the object, where every other recovery here searches within prose.
        if allow_grammar_json:
            action = _grammar_json_action(
                text, allowed_tools, allow_abstain=allow_abstain)
            if action is not None:
                # The format label follows the grammar that produced it. v2 admits the
                # abstention sentinel and v1 does not, so pooling their rows would
                # compare a model that could decline against one that could not.
                fmt = "grammar_json_v2" if allow_abstain else "grammar_json_v1"

        if action is None:
            action = _snake_text_call(text, allowed_tools)

        if action is None:
            action = _camel_text_call(text, allowed_tools)

        if action is None and allow_tools_wrapper:
            action = _tools_text_call(text, allowed_tools)
            fmt = "text_tools_v1"

        if action is not None:
            recovered.append((action, fmt))

    # More than one recovered action is ambiguous.
    if len(recovered) != 1:
        return None, "none"

    return recovered[0]


def compare(local: Action | None, cloud: Action | None) -> Agreement:
    """Grade one local attempt against the cloud's own answer.

    An `unscorable` verdict is a real outcome and is not folded into disagreement.
    Counting "the model wrote prose" as "the model was wrong" would inflate the
    disagreement rate with turns that were never comparable in the first place.
    """
    if local is None or cloud is None:
        return Agreement.UNSCORABLE
    if local.tool != cloud.tool or local.target != cloud.target:
        return Agreement.DIFFERENT_ACTION
    if local.normalised_args() == cloud.normalised_args():
        return Agreement.SAME_ACTION_SAME_ARGS
    return Agreement.SAME_ACTION


class StreamActionAccumulator:
    """Reassemble only the tool call from an SSE stream. Never the prose.

    Feed it raw event bytes as they relay past. It watches for a tool-call block opening,
    collects the argument fragments that follow, and ignores every text delta — so the
    natural-language output of the model is not held even transiently. That is a stronger
    property than buffering the response and extracting afterwards, and it is the reason
    this component's privacy surface is a file path rather than a customer's source code.
    """

    def __init__(self, max_bytes: int = MAX_ARGS_BYTES) -> None:
        self.max_bytes = max_bytes
        self._tool: str | None = None
        self._fragments: list[str] = []
        self._size = 0
        self.truncated = False

    def feed(self, chunk: bytes | str) -> None:
        """Consume relayed bytes. Must never raise: this sits beside a live stream."""
        try:
            text = chunk.decode("utf-8", "replace") if isinstance(chunk, bytes) else chunk
            for line in text.splitlines():
                line = line.strip()
                if line.startswith("data:"):
                    self._event(line[5:].strip())
        except Exception:  # noqa: BLE001
            return

    def _event(self, blob: str) -> None:
        if not blob or blob == "[DONE]":
            return
        try:
            event = json.loads(blob)
        except (ValueError, TypeError):
            return
        if not isinstance(event, dict):
            return

        kind = event.get("type")

        if kind == "content_block_start":                             # Anthropic
            block = event.get("content_block") or {}
            if isinstance(block, dict) and block.get("type") == "tool_use":
                self._tool = block.get("name")
                partial = block.get("input")
                if isinstance(partial, dict) and partial:
                    self._add(json.dumps(partial))
        elif kind == "content_block_delta":
            delta = event.get("delta") or {}
            # `text_delta` is deliberately ignored. The prose is not our business.
            if isinstance(delta, dict) and delta.get("type") == "input_json_delta":
                self._add(str(delta.get("partial_json") or ""))

        for choice in event.get("choices") or []:                     # OpenAI
            if not isinstance(choice, dict):
                continue
            delta = choice.get("delta") or {}
            for call in (delta.get("tool_calls") or []):
                if not isinstance(call, dict):
                    continue
                fn = call.get("function") or {}
                if fn.get("name"):
                    self._tool = fn["name"]
                if fn.get("arguments"):
                    self._add(str(fn["arguments"]))

    def _add(self, fragment: str) -> None:
        if self.truncated:
            return
        if self._size + len(fragment) > self.max_bytes:
            # Stop rather than grow. A truncated comparison is reported as unscorable,
            # which is honest; an unbounded one is a memory bug on someone's laptop.
            self.truncated = True
            return
        self._fragments.append(fragment)
        self._size += len(fragment)

    def action(self) -> Action | None:
        if self._tool is None:
            return None
        return _action(self._tool, "".join(self._fragments), truncated=self.truncated)
