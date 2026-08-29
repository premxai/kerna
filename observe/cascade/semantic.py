"""Did the local model choose an *equivalent* way to do the same thing?

Exact action match asks whether a 7B model reproduces Claude's exact button presses.
That is not the routing question. The first analysed cohort made the difference concrete:
10 clean comparisons, 10 `different_action`, and reading the pairs by hand showed roughly
three genuine semantic matches, three real divergences, three too obscured to judge, and
one where the intent was right and the command would not have run.

    Claude:  Bash("grep -rn EXPLORE evals/m0/")
    Qwen:    Grep(pattern="explore", path="evals/m0/")

`different_action` is *true* and useless. Both searched the same tree for the same string.

## Components first, and one named interpretation

A semantic matcher is the most dangerous thing in this repository, because it is a knob
that makes our own number go up. Loosen it enough and everything agrees. Every previous
number that flattered us came from a measurement that had quietly moved its own goalposts
— the self-authored corpus (023), the ignored grammar (045), the prevention metric that
counted broken runs as successes.

So **intent**, **resource** and **query** are recorded separately, along with which
canonicalisation rule fired. A rate computed from those can be *recomputed* by someone
who disagrees with the threshold.

`semantic_equivalent` is also recorded, and it is not nothing: it is **the `semantic-v1`
derived interpretation, not ground truth.** Stating it that way rather than claiming the
module reaches no verdict is the honest description — it reaches one, under a named and
frozen rule set.

## The version is the point

`semantic-v1` is frozen. Its rule is:

    intent_match AND (resource_overlap OR query_match)

with UNKNOWN/UNKNOWN and EXECUTE/EXECUTE excluded from `intent_match`.

Every scored row carries `semantic_policy`. Without it, someone tightening the rule in
six weeks would leave old and new rows both saying `semantic_equivalent` while meaning
different things, and the two would pool — the exact failure `class_version` exists to
prevent and that the ignored grammar already cost this project once.

**Never change v1 in place.** A different rule is `semantic-v2`, and rows from the two
are never compared without saying so.

## The canonicalisation is deliberately small

Only equivalences that are unambiguous in the shell:

    Bash(grep|rg|egrep)        -> SEARCH        Grep      -> SEARCH
    Bash(cat|head|tail|sed -n) -> READ          Read      -> READ
    Bash(ls|find|tree)         -> LIST          Glob      -> LIST
    Edit / Write / NotebookEdit -> WRITE
    anything else in Bash      -> EXECUTE

`sed -n '455,530p' file` is a read; `sed -i` is a write, and conflating them would let a
destructive edit match a harmless inspection. Where a command is not clearly one of
these, the intent is EXECUTE and the pair simply does not match on intent — silence is
the correct output for a case the rules do not cover.

## What it deliberately does not claim

Nothing here says an action would *work*. One pair in the cohort had the right intent,
the right file, and a command (`cat file:485-507`) that would have failed. Validity is
what the sandbox check measures (`check.py`), and pretending a semantic match implies a
working command is exactly the overclaim this module exists to avoid.
"""

from __future__ import annotations

import re
import shlex
from dataclasses import dataclass, field
from enum import Enum
from typing import Any


# Frozen. Changing any rule below -- the intent tables, the exclusion of UNKNOWN and
# EXECUTE, or the definition of `equivalent` -- means a new version, never an edit to
# this one. Rows carry it so cohorts cannot silently pool across rule changes.
SEMANTIC_POLICY = "semantic-v1"


class Intent(str, Enum):
    SEARCH = "search"
    READ = "read"
    LIST = "list"
    WRITE = "write"
    EXECUTE = "execute"
    UNKNOWN = "unknown"


# Tool name -> intent, for tools whose name already says what they do.
_TOOL_INTENT: dict[str, Intent] = {
    "grep": Intent.SEARCH,
    "read": Intent.READ,
    "glob": Intent.LIST,
    "ls": Intent.LIST,
    "edit": Intent.WRITE,
    "write": Intent.WRITE,
    "notebookedit": Intent.WRITE,
}

# Shell command -> intent. Only the unambiguous ones.
_SHELL_INTENT: dict[str, Intent] = {
    "grep": Intent.SEARCH,
    "rg": Intent.SEARCH,
    "egrep": Intent.SEARCH,
    "fgrep": Intent.SEARCH,
    "ack": Intent.SEARCH,
    "cat": Intent.READ,
    "head": Intent.READ,
    "tail": Intent.READ,
    "more": Intent.READ,
    "less": Intent.READ,
    "ls": Intent.LIST,
    "dir": Intent.LIST,
    "find": Intent.LIST,
    "tree": Intent.LIST,
}

# Flags that carry a value, so the value is not mistaken for a search term or a path.
_VALUE_FLAGS = {"--include", "--exclude", "--glob", "-e", "-m", "-A", "-B", "-C"}

_PATHISH = re.compile(r"[/\\.]")


@dataclass(frozen=True)
class Reading:
    """What one proposed action was trying to do."""

    intent: Intent
    query: str | None          # the string being searched for, if any
    resources: tuple[str, ...]  # paths or globs the action touches
    rule: str                   # which canonicalisation produced this

    def as_dict(self) -> dict[str, Any]:
        return {
            "intent": self.intent.value,
            "query": self.query,
            "resources": list(self.resources),
            "rule": self.rule,
        }


def _split_shell(command: str) -> list[list[str]]:
    """Shell text as a list of simple commands, split on pipes and separators."""
    try:
        tokens = shlex.split(command, posix=True)
    except ValueError:
        tokens = command.split()

    segments: list[list[str]] = [[]]
    for token in tokens:
        if token in ("|", "&&", ";", "||"):
            segments.append([])
        else:
            segments[-1].append(token)
    return [s for s in segments if s]


def _is_read_sed(argv: list[str]) -> bool:
    """`sed -n '1,20p'` inspects; `sed -i` edits. Conflating them would let a destructive
    edit match a harmless read, which is the one direction this must never fail in."""
    return argv and argv[0] == "sed" and "-n" in argv and not any(
        a.startswith("-i") for a in argv)


def _read_shell(command: str) -> Reading:
    segments = _split_shell(command)

    # `cd X && real-command` is the shape almost every Claude Code Bash call takes, so
    # leading directory changes are skipped rather than classified.
    meaningful = [s for s in segments
                  if s and s[0] not in ("cd", "head", "tail") or (
                      s and s[0] in ("head", "tail") and len(segments) == 1)]
    if not meaningful:
        meaningful = segments

    for argv in meaningful:
        if not argv:
            continue
        program = argv[0].split("/")[-1].split("\\")[-1]

        if _is_read_sed(argv):
            return Reading(Intent.READ, None, _paths(argv[1:]), "shell:sed-n")

        intent = _SHELL_INTENT.get(program)
        if intent is None:
            continue

        if intent is Intent.SEARCH:
            query, paths = _search_operands(argv[1:])
            return Reading(intent, query, paths, f"shell:{program}")
        return Reading(intent, None, _paths(argv[1:]), f"shell:{program}")

    return Reading(Intent.EXECUTE, None, (), "shell:unclassified")


def _search_operands(rest: list[str]) -> tuple[str | None, tuple[str, ...]]:
    """The pattern and the paths from a grep-like argument list.

    The first non-flag operand is the pattern; the remainder are paths. Flags that take a
    value consume the next token, so `--include=*.py` and `-e pattern` do not masquerade
    as the search term.
    """
    query: str | None = None
    paths: list[str] = []
    skip = False

    for token in rest:
        if skip:
            skip = False
            continue
        if token.startswith("-"):
            base = token.split("=", 1)[0]
            if base in _VALUE_FLAGS and "=" not in token:
                skip = True
            continue
        if query is None:
            query = token
        else:
            paths.append(token)

    return query, tuple(paths)


def _paths(rest: list[str]) -> tuple[str, ...]:
    out = []
    skip = False
    for token in rest:
        if skip:
            skip = False
            continue
        if token.startswith("-"):
            base = token.split("=", 1)[0]
            if base in _VALUE_FLAGS and "=" not in token:
                skip = True
            continue
        out.append(token)
    return tuple(out)


def read_action(tool: str, args: dict[str, Any] | None) -> Reading:
    """Canonicalise one proposed action into intent, query and resources."""
    args = args or {}
    name = str(tool or "").strip().lower()

    if name == "bash" or name == "powershell":
        command = args.get("command")
        if isinstance(command, str) and command.strip():
            return _read_shell(command)
        return Reading(Intent.EXECUTE, None, (), "shell:no-command")

    intent = _TOOL_INTENT.get(name, Intent.UNKNOWN)
    query = None
    if intent is Intent.SEARCH:
        for key in ("pattern", "query", "regex"):
            value = args.get(key)
            if isinstance(value, str) and value.strip():
                query = value.strip()
                break

    resources = []
    for key in ("file_path", "notebook_path", "path", "filename", "file", "glob_pattern"):
        value = args.get(key)
        if isinstance(value, str) and value.strip():
            resources.append(value.strip())
    if intent is Intent.LIST and not resources:
        value = args.get("pattern")
        if isinstance(value, str) and value.strip():
            resources.append(value.strip())

    return Reading(intent, query, tuple(resources), f"tool:{name or 'unnamed'}")


# ------------------------------------------------------------------ comparison


def _norm_path(path: str) -> str:
    return path.replace("\\", "/").strip().strip("./").lower()


def _resource_overlap(a: tuple[str, ...], b: tuple[str, ...]) -> bool:
    """Whether two actions touch a common file or tree.

    Containment counts in either direction: `evals/m0/` and `evals/m0/cascade/x.py` are
    the same target at different resolutions, which is precisely the kind of near-miss
    exact matching throws away. A bare `.` or `./` matches nothing on purpose -- it means
    "the whole repository", and letting it overlap with everything would make the
    resource test vacuous.
    """
    left = {_norm_path(p) for p in a if _norm_path(p)}
    right = {_norm_path(p) for p in b if _norm_path(p)}
    if not left or not right:
        return False
    for x in left:
        for y in right:
            if x == y or x.startswith(y + "/") or y.startswith(x + "/"):
                return True
    return False


def _query_match(a: str | None, b: str | None) -> bool | None:
    """Case-insensitive, and substring in either direction.

    None when either side had no query, so "not comparable" never reads as "did not
    match". `EXPLORE` and `explore` are the same search; so are `vram` and `free_vram`.
    """
    if a is None or b is None:
        return None
    x, y = a.strip().lower(), b.strip().lower()
    if not x or not y:
        return None
    return x == y or x in y or y in x


@dataclass
class SemanticVerdict:
    """Components, not a conclusion.

    Deliberately not a boolean. A single semantic-match flag is a threshold baked into
    the data, and this project has been burned three times by a number whose definition
    moved. Recording the parts means an analysis that disagrees can recompute rather than
    re-run.
    """

    intent_match: bool
    resource_overlap: bool
    query_match: bool | None
    cloud: Reading
    local: Reading

    @property
    def equivalent(self) -> bool:
        """The `semantic-v1` interpretation: same intent, and either the same target or
        the same search term.

        A derived reading under a named, frozen rule set -- not ground truth, and never
        recorded without the components it came from.
        """
        if not self.intent_match:
            return False
        return bool(self.resource_overlap) or self.query_match is True

    def as_dict(self) -> dict[str, Any]:
        return {
            "semantic_policy": SEMANTIC_POLICY,
            "semantic_intent_match": self.intent_match,
            "semantic_resource_overlap": self.resource_overlap,
            "semantic_query_match": self.query_match,
            "semantic_equivalent": self.equivalent,
            "cloud_intent": self.cloud.intent.value,
            "local_intent": self.local.intent.value,
            "cloud_rule": self.cloud.rule,
            "local_rule": self.local.rule,
        }


def compare_semantic(
    cloud_tool: str,
    cloud_args: dict[str, Any] | None,
    local_tool: str,
    local_args: dict[str, Any] | None,
) -> SemanticVerdict:
    cloud = read_action(cloud_tool, cloud_args)
    local = read_action(local_tool, local_args)

    # UNKNOWN never matches UNKNOWN. Two actions we failed to classify are not evidence
    # of agreement, and treating them as such would make the rate climb precisely where
    # the rules are weakest.
    intent_match = (
        cloud.intent is local.intent
        and cloud.intent not in (Intent.UNKNOWN, Intent.EXECUTE)
    )

    return SemanticVerdict(
        intent_match=intent_match,
        resource_overlap=_resource_overlap(cloud.resources, local.resources),
        query_match=_query_match(cloud.query, local.query),
        cloud=cloud,
        local=local,
    )
