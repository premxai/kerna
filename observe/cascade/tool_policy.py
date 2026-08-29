from __future__ import annotations

import json

from typing import Any


TOOL_POLICIES = ("full", "core-v1")

# Frozen before looking at Qwen agreement.
#
# This is the ordinary coding surface needed to inspect and modify a repository.
# If Claude chooses anything outside this menu, that turn is not a disagreement:
# the local model was never offered Claude's action, so the comparison is unscorable.
CORE_V1 = frozenset({
    "Read",
    "Glob",
    "Grep",
    "Bash",
    "PowerShell",
    "Edit",
    "Write",
    "NotebookEdit",
})


def tool_name(tool: Any) -> str | None:
    """Return a tool name from either Anthropic or OpenAI tool schema."""
    if not isinstance(tool, dict):
        return None

    # Anthropic:
    # {"name": "Read", "input_schema": {...}}
    name = tool.get("name")
    if isinstance(name, str) and name:
        return name

    # OpenAI:
    # {"type": "function", "function": {"name": "Read", ...}}
    function = tool.get("function")
    if isinstance(function, dict):
        name = function.get("name")
        if isinstance(name, str) and name:
            return name

    return None


def filter_tools(
    tools: list[Any],
    policy: str,
) -> list[dict[str, Any]]:
    """Return the exact tool menu offered to the local shadow model.

    `full` means every ordinary tool currently supported by the local experiment.
    MCP/plugin tools remain excluded because they were already excluded from the local
    shadow path before this policy existed.

    `core-v1` is a deliberately frozen, small coding menu.
    """
    if policy not in TOOL_POLICIES:
        raise ValueError(f"unknown tool policy: {policy}")

    ordinary: list[dict[str, Any]] = []

    for tool in tools:
        if not isinstance(tool, dict):
            continue

        name = tool_name(tool)
        if not name:
            continue

        if name.startswith("mcp__"):
            continue

        ordinary.append(tool)

    if policy == "full":
        return ordinary

    return [
        tool
        for tool in ordinary
        if tool_name(tool) in CORE_V1
    ]


def offered_tool_names(
    payload: dict[str, Any],
    policy: str,
) -> frozenset[str]:
    """Names available to the local model under this policy."""
    tools = payload.get("tools")
    if not isinstance(tools, list):
        return frozenset()

    return frozenset(
        name
        for tool in filter_tools(tools, policy)
        if (name := tool_name(tool)) is not None
    )


def cloud_action_available(
    payload: dict[str, Any],
    cloud_tool: str | None,
    policy: str,
) -> bool | None:
    """Whether Claude's chosen tool existed in the local action space.

    None means the verdict cannot be established: either Claude produced no tool
    action, or the captured payload does not contain a usable tool menu.

    False is reserved for the stronger claim that a tool menu was present and the
    selected policy definitely did not offer Claude's chosen action.
    """
    if cloud_tool is None:
        return None

    tools = payload.get("tools")
    if not isinstance(tools, list):
        return None

    declared = [
        tool_name(tool)
        for tool in tools
        if tool_name(tool) is not None
    ]

    if not declared:
        return None

    return cloud_tool in offered_tool_names(payload, policy)



def _without_cache_control(value: Any) -> Any:
    """Match the local shadow request's metadata cleanup for measurement."""
    if isinstance(value, dict):
        return {
            key: _without_cache_control(item)
            for key, item in value.items()
            if key != "cache_control"
        }
    if isinstance(value, list):
        return [_without_cache_control(item) for item in value]
    return value


def _schema_chars(tools: list[dict[str, Any]]) -> int:
    """Deterministic UTF-8 JSON character count, not a token estimate.

    This field exists to expose a context component our earlier traffic counters missed.
    It must never be converted to tokens with a chars/4 heuristic; tokenizer-measured
    request size remains a separate measurement.
    """
    cleaned = _without_cache_control(tools)
    return len(json.dumps(
        cleaned,
        ensure_ascii=False,
        separators=(",", ":"),
    ))


def tool_metrics(
    payload: dict[str, Any],
    policy: str,
) -> dict[str, int]:
    """Measure the tool surface before and after the local-only tool policy.

    The original measurement keeps every named tool, including MCP integrations.
    The local measurement is the exact policy-selected menu.

    Both schema-char measurements remove cache_control so their difference isolates
    tool-menu selection instead of mixing selection with Anthropic cache annotations.
    """
    tools = payload.get("tools")

    if not isinstance(tools, list):
        return {
            "original_tool_count": 0,
            "local_tool_count": 0,
            "original_tool_schema_chars": 0,
            "local_tool_schema_chars": 0,
        }

    original = [
        tool
        for tool in tools
        if isinstance(tool, dict) and tool_name(tool) is not None
    ]

    local = filter_tools(original, policy)

    return {
        "original_tool_count": len(original),
        "local_tool_count": len(local),
        "original_tool_schema_chars": _schema_chars(original),
        "local_tool_schema_chars": _schema_chars(local),
    }
