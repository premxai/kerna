"""What kind of work is this turn, and is the tree in a state worth checking? (P1.2-A)

Nothing produces `task_class` today. The dispatcher takes it as a parameter, production
passes `None`, and `None` routes to cloud — so this is the component that decides whether
anything is ever eligible at all.

It also carries the finding from P1.1, which arrived after this component was specified
and changed what it has to compute.

## The P1.1 finding, and why it lives here

The first clean capture ran:

    Glob -> Read -> Edit x8 -> Bash(tests) -> Read -> Read

**Eight consecutive edits, then one test run.** Each of those edits is behaviour-checkable
in principle: apply it, run the suite. None of them is checkable *in practice*, because
mid-sequence the tree is deliberately in a broken intermediate state — running the suite
after edit 3 reports failures caused by edits 1 and 2 being unfinished. The check is
available and its answer is worthless.

> Checkability is not a property of a turn. It is a property of a **position in a
> sequence**, and only an edit at a natural checkpoint is cleanly verifiable.

The routing consequence is direct: a policy of "attempt every eligible turn" would route
edits 2 through 8 and validate every one of them against a tree that cannot pass. So this
module computes `edits_since_check` — **how many edits the agent has made since it last
saw a gate outcome** — and that number, not the tool name, is what says whether a check
run now would mean anything.

It is computable at decision time, which is the part that makes it usable: every edit the
agent has proposed and every result it has received are already in the conversation we are
being asked to complete.

## Two rules that are about the ledger, not the classifier

**Stability over accuracy.** The Autonomy Ledger is a long-run accumulator with Wilson
bounds per class. A class whose meaning drifts silently invalidates every observation
recorded under it — strictly worse than a coarse class that stays put. So classes are
derived from structural fields only, never from prompt text (the system prompt is dynamic,
and *how task identity survives a dynamic system prompt* is an open question we should not
answer by accident), and `CLASS_VERSION` is stamped on every row so a change to this file
is a visible break rather than quiet corruption.

**Bounded cardinality.** A classifier that emits a fresh class per request produces a
ledger that promotes nothing, because no class ever accumulates the samples to earn a
bound. `CLASSES` is closed and a test asserts it.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

# Bump when the meaning of any class changes. Ledger rows carry it, and rows from two
# versions are never pooled — that is the whole reason the field exists.
CLASS_VERSION = "1"

EDIT = "edit"
READ = "read"
COMMAND = "command"
SEARCH = "search"
UNKNOWN = "unknown"

CLASSES: frozenset[str] = frozenset({EDIT, READ, COMMAND, SEARCH, UNKNOWN})

# Coarse on purpose. Every one of these buckets must be able to accumulate hundreds of
# observations on one developer's machine inside a week, or the ledger never promotes it.
_TOOL_CLASSES: dict[str, str] = {
    "Edit": EDIT, "Write": EDIT, "NotebookEdit": EDIT, "MultiEdit": EDIT,
    "Read": READ, "NotebookRead": READ,
    "Bash": COMMAND, "Shell": COMMAND, "Run": COMMAND,
    "Glob": SEARCH, "Grep": SEARCH, "WebSearch": SEARCH, "WebFetch": SEARCH,
}

_NAME_HINTS: tuple[tuple[tuple[str, ...], str], ...] = (
    (("edit", "write", "patch", "apply", "replace"), EDIT),
    (("read", "cat", "open", "view"), READ),
    (("bash", "shell", "exec", "run", "command", "terminal"), COMMAND),
    (("search", "grep", "glob", "find", "list", "fetch"), SEARCH),
)


def _words(name: str) -> list[str]:
    """Split a tool name into lowercase words: `SmartPatchTool` -> smart, patch, tool.

    Bare substring matching put `Frobnicate` in the READ class, because it contains
    `cat`. That is funny once and expensive forever: a misclassified tool sends its
    observations into the wrong ledger bucket, where they are indistinguishable from real
    evidence and quietly move a promotion threshold. `Truncate`, `Locate` and `Listen`
    would all have done the same.
    """
    out: list[str] = []
    current: list[str] = []
    for ch in name:
        if ch in "_-. ":
            if current:
                out.append("".join(current).lower())
                current = []
        elif ch.isupper() and current:
            out.append("".join(current).lower())
            current = [ch]
        else:
            current.append(ch)
    if current:
        out.append("".join(current).lower())
    return out

# Markers that a tool result carried a gate outcome — the agent ran something that says
# whether the code works. Shared with the recorder's shape detection in spirit, kept
# separate in fact, because this list decides *routing* and that one decides a label.
_CHECK_MARKERS: tuple[str, ...] = (
    "passed", "failed", "test session starts", "assertionerror",
    "tests ran", "ok (", "fail:", "error:", "traceback (most recent call last)",
)


@dataclass(frozen=True)
class Turn:
    """What the dispatcher needs to know about one request."""

    task_class: str
    edits_since_check: int
    saw_check_result: bool          # the agent has just been handed a gate outcome
    version: str = CLASS_VERSION

    @property
    def at_checkpoint(self) -> bool:
        """Would a check run right now mean anything?

        True when the tree has not been edited since the last gate outcome — the only
        position where running the suite tests *this* change rather than the wreckage of
        an unfinished sequence. P1.1 measured one such position in thirteen turns.
        """
        return self.edits_since_check == 0


def _blocks(message: dict[str, Any]) -> list[dict[str, Any]]:
    content = message.get("content")
    return [b for b in content if isinstance(b, dict)] if isinstance(content, list) else []


def _tool_calls(message: dict[str, Any]) -> list[str]:
    """Tool names invoked in this message, in both dialects."""
    names = [str(b.get("name")) for b in _blocks(message)
             if b.get("type") == "tool_use" and b.get("name")]
    calls = message.get("tool_calls")
    if isinstance(calls, list):
        names += [str((c.get("function") or {}).get("name")) for c in calls
                  if isinstance(c, dict) and (c.get("function") or {}).get("name")]
    return names


def _result_texts(message: dict[str, Any]) -> list[str]:
    """Tool result payloads in this message, in both dialects."""
    texts: list[str] = []
    if message.get("role") == "tool":
        texts.append(_as_text(message.get("content")))
    texts += [_as_text(b.get("content")) for b in _blocks(message)
              if b.get("type") == "tool_result"]
    return texts


def _as_text(value: Any) -> str:
    if isinstance(value, str):
        return value
    if isinstance(value, list):
        return "\n".join(_as_text(v) for v in value)
    if isinstance(value, dict):
        return _as_text(value.get("text") or value.get("content") or "")
    return ""


def class_of(tool: str | None) -> str:
    """One tool name -> one of a closed set of classes."""
    if not tool:
        return UNKNOWN
    if tool in _TOOL_CLASSES:
        return _TOOL_CLASSES[tool]
    words = set(_words(tool))
    for needles, cls in _NAME_HINTS:
        if words & set(needles):
            return cls
    return UNKNOWN


def carries_check_result(text: str) -> bool:
    """Did this tool result tell the agent whether the code works?"""
    low = text.lower()
    return any(marker in low for marker in _CHECK_MARKERS)


def classify(payload: dict[str, Any]) -> Turn | None:
    """Classify one request, or return None when it cannot be classified.

    `None` is not a failure mode to be minimised. The dispatcher already sends `None` to
    cloud, so an unclassifiable request costs the customer nothing and costs us one
    unexplored turn — **fail closed on classification, fail open on serving.** A
    classifier that guesses to raise its coverage is trading the ledger's integrity for a
    number nobody asked for.
    """
    messages = [m for m in (payload.get("messages") or []) if isinstance(m, dict)]
    if not messages:
        return None

    # Walk forward, tracking edits against the last gate outcome. Forward rather than
    # backward because the count is a running total, and the last result that carried a
    # verdict is where the clock resets.
    edits_since_check = 0
    saw_check = False
    last_tool: str | None = None

    for message in messages:
        for name in _tool_calls(message):
            last_tool = name
            if class_of(name) is EDIT:
                edits_since_check += 1
        for text in _result_texts(message):
            if carries_check_result(text):
                edits_since_check = 0
                saw_check = True

    task_class = class_of(last_tool)
    if task_class is UNKNOWN and last_tool is None:
        return None                       # no action taken yet: nothing to attribute

    return Turn(task_class=task_class,
                edits_since_check=edits_since_check,
                saw_check_result=saw_check)
