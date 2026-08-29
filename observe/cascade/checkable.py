"""How much agent work carries a check we could run *before* answering? (P1.1)

This is the number that sizes the product, and it sits in a gap between two things that
both look answered.

**Decision 036** settled eligibility: a turn is agentic when it declares tools and carries
a prior tool result. Measured, and it replaced a rule that routed nothing.

**Decision 029** requires that a routed task carry a **pre-declared check**. The loop
harness satisfies 029 by reading `item.check_spec` — which a benchmark item has and **a
live request does not.**

C1-observe found the gate outcome is available in real traffic: 718 of 718 tool calls
return their result in the very next request. But it arrives **one turn late.** Perfect
for the ledger, which scores after the fact. Useless for serving, which must decide before
returning bytes.

So the servable slice is some subset of the eligible slice, and this module measures which.

## Why this needs no local model, no oracle, and no product

Whether a turn carries a runnable check is a property of the **traffic** — of what the
agent is being asked to do — not of the model, the machine, or the routing. So it is
measurable from a recording, three days of work, ahead of the ten-day MVP it can reshape
or cancel. That ordering is the main argument of docs/EXPLORE-MVP.md.

## What is actually being classified

`last_tool` on request *i* is the action the model chose at turn *i-1*, so the
distribution of `last_tool` over a capture **is** the distribution of actions the agent
took — no joining across requests required. The measurement is therefore retrospective and
honest about it: it reports the *ceiling* of what routing could have served, not a
prediction of what it would serve.

## The three verdicts, and why the split is not cosmetic

  * `checkable_behaviour` — we could run something that says whether it **works**
  * `checkable_form` — we could only confirm it is **well-formed**
  * `unchecked` — nothing runnable before the answer goes back

That line is the one this project has now measured three times: citations at 33.3%,
derivability at 34.8%, prompt-only prediction at chance. Form checks have failed every
time and behaviour checks have worked every time. Reporting a single "checkable" number
would merge the two and destroy the finding.
"""

from __future__ import annotations

import argparse
import json
import math
from collections import Counter
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterable

from ..console import emit

BEHAVIOUR = "checkable_behaviour"
FORM = "checkable_form"
UNCHECKED = "unchecked"

# The taxonomy, with the reasoning attached to each entry rather than kept in someone's
# head. The question for every row is not "is this tool important" but "before returning
# the model's choice, could we run something that says whether the choice was good?"
TOOL_VERDICTS: dict[str, tuple[str, str]] = {
    # Writing code is the case the whole product was designed around: apply the edit to a
    # scratch copy, run the affected tests, and you know whether it works before anyone
    # sees it. Expensive, and the only kind of check that has ever discriminated.
    "Edit":         (BEHAVIOUR, "apply to a scratch copy and run the affected tests"),
    "Write":        (BEHAVIOUR, "apply to a scratch copy and run the affected tests"),
    "NotebookEdit": (BEHAVIOUR, "apply to a scratch copy and run the affected tests"),

    # A command can be parsed and matched against an allowlist. Whether it is the *right*
    # command cannot be known without running it, and running it has side effects — so
    # this is form, and calling it behaviour would be the flattering mistake.
    "Bash":         (FORM, "the command parses and is on an allowlist"),

    # Reads: we can confirm the path or pattern is valid. Whether this was the right file
    # to open is not checkable by any means available before answering.
    "Read":         (FORM, "the path exists"),
    "Glob":         (FORM, "the pattern is well-formed"),
    "Grep":         (FORM, "the pattern compiles"),
    "WebFetch":     (FORM, "the URL parses"),
    "WebSearch":    (FORM, "the query is non-empty"),

    # Delegation and planning produce no artefact to verify against anything.
    "Task":         (UNCHECKED, "delegates; the work happens elsewhere"),
    "Agent":        (UNCHECKED, "delegates; the work happens elsewhere"),
    "TodoWrite":    (UNCHECKED, "bookkeeping, no correctness to establish"),
    "ExitPlanMode": (UNCHECKED, "control flow, not work"),
}

# Suffix rules for tools we have not seen by name. Deliberately conservative: an unknown
# tool is `unchecked`, never optimistically promoted, because every wrong guess here
# inflates the headline number in the direction we would like it to go.
_SUFFIX_HINTS: tuple[tuple[tuple[str, ...], str, str], ...] = (
    (("edit", "write", "patch", "apply"), BEHAVIOUR, "writes code; testable in a scratch copy"),
    (("read", "get", "list", "search", "find", "grep", "glob"), FORM, "read-only; arguments checkable"),
    (("run", "exec", "bash", "shell", "command"), FORM, "executable; arguments checkable only"),
)


def classify_action(tool: str | None) -> tuple[str, str]:
    """One action -> (verdict, why). Unknown tools are never optimistically promoted."""
    if not tool:
        return UNCHECKED, "no tool call — prose, or the turn produced no action"
    if tool in TOOL_VERDICTS:
        return TOOL_VERDICTS[tool]
    low = tool.lower()
    for needles, verdict, why in _SUFFIX_HINTS:
        if any(n in low for n in needles):
            return verdict, f"unrecognised tool, matched on name: {why}"
    return UNCHECKED, "unrecognised tool, no rule matched — counted as unchecked"


def wilson(successes: int, total: int, z: float = 1.96) -> tuple[float, float]:
    if total == 0:
        return 0.0, 0.0
    p = successes / total
    d = 1 + z * z / total
    centre = (p + z * z / (2 * total)) / d
    half = z * math.sqrt(p * (1 - p) / total + z * z / (4 * total * total)) / d
    return max(0.0, centre - half), min(1.0, centre + half)


def is_agentic_row(row: dict[str, Any]) -> bool:
    """The recorded equivalent of `dispatcher.is_agentic` — tools AND a prior result."""
    return bool(row.get("n_tools", 0)) and bool(row.get("has_tool_results"))


def load_rows(log: Path, *, clean_only: bool = True) -> list[dict[str, Any]]:
    """Requests from a traffic log, with the client's blocking retries removed.

    Before the `accept-encoding` fix the client could not parse our relayed stream and
    silently retried each turn blocking, so a raw capture double-counts every turn of the
    affected window. Counting them once produced "59% of agentic turns stream", a number
    describing no client that ever existed — and the fix landed mid-run, so the per-run
    header does not separate them. The duplicates are detectable by shape, so they are
    removed here rather than left to poison a second measurement.
    """
    rows = [json.loads(line) for line in log.read_text(encoding="utf-8").splitlines()
            if line.strip()]
    requests = [r for r in rows if r.get("record") == "request"]
    if not clean_only:
        return requests

    duplicates: set[int] = set()
    for i in range(len(requests) - 1):
        a, b = requests[i], requests[i + 1]
        if (a.get("stream") and not b.get("stream")
                and a.get("n_messages") == b.get("n_messages")
                and a.get("total_chars") == b.get("total_chars")):
            duplicates.add(i + 1)
    return [r for i, r in enumerate(requests) if i not in duplicates]


@dataclass
class Slice:
    total_requests: int
    agentic: int
    verdicts: Counter
    tools: Counter
    shapes: Counter
    unknown_tools: Counter
    missing_tool_field: int

    @property
    def behaviour(self) -> int:
        return self.verdicts[BEHAVIOUR]

    def share(self, verdict: str) -> tuple[float, float, float]:
        n = self.agentic
        k = self.verdicts[verdict]
        lo, hi = wilson(k, n)
        return (k / n if n else 0.0), lo, hi


def measure(rows: Iterable[dict[str, Any]]) -> Slice:
    rows = list(rows)
    agentic = [r for r in rows if is_agentic_row(r)]
    verdicts: Counter = Counter()
    tools: Counter = Counter()
    shapes: Counter = Counter()
    unknown: Counter = Counter()
    missing = 0

    for row in agentic:
        if "last_tool" not in row:
            # The field postdates the capture. Counted and reported, never guessed —
            # a measurement quietly taken over rows that cannot answer the question is
            # the failure mode this whole file exists downstream of.
            missing += 1
            continue
        tool = row.get("last_tool")
        verdict, _ = classify_action(tool)
        verdicts[verdict] += 1
        tools[tool or "<none>"] += 1
        shapes[row.get("last_result_shape") or "<none>"] += 1
        if tool and tool not in TOOL_VERDICTS:
            unknown[tool] += 1

    return Slice(len(rows), len(agentic), verdicts, tools, shapes, unknown, missing)


GATE_BEHAVIOUR_LB = 0.15


def render(sl: Slice) -> str:
    out: list[str] = []
    add = out.append
    add("# P1.1 — the checkable slice")
    add("")

    if sl.missing_tool_field:
        add(f"> **{sl.missing_tool_field} of {sl.agentic} agentic requests predate the "
            f"`last_tool` field and cannot be classified.** They are excluded, not "
            f"guessed. Re-capture with the current recorder before treating anything "
            f"below as the answer.")
        add("")

    scored = sl.agentic - sl.missing_tool_field
    add(f"- requests in the clean window: **{sl.total_requests}**")
    add(f"- agentic (tools declared **and** a prior tool result): **{sl.agentic}**")
    add(f"- classifiable: **{scored}**")
    add("")

    if not scored:
        add("**Nothing classifiable. This measurement has not been made.**")
        return "\n".join(out)

    add("| verdict | n | share of agentic | 95% CI |")
    add("|---|---|---|---|")
    for verdict in (BEHAVIOUR, FORM, UNCHECKED):
        k = sl.verdicts[verdict]
        lo, hi = wilson(k, scored)
        add(f"| `{verdict}` | {k} | **{k / scored:.1%}** | {lo:.1%} – {hi:.1%} |")
    add("")

    lo, hi = wilson(sl.behaviour, scored)
    verdict = "PASSES" if lo >= GATE_BEHAVIOUR_LB else "FAILS"
    add(f"**Gate — behaviour-checkable ≥ {GATE_BEHAVIOUR_LB:.0%} at the lower bound: "
        f"{verdict}.**  ({sl.behaviour}/{scored}, lb {lo:.1%})")
    if lo < GATE_BEHAVIOUR_LB:
        add("")
        add("Below this bar the serving product has no slice worth the engineering, and "
            "EXPLORE mode is a measurement instrument for selling an assessment rather "
            "than a step toward routing.")
    add("")

    add("## Which tools the agent actually used")
    add("")
    add("| tool | n | verdict | why |")
    add("|---|---|---|---|")
    for tool, n in sl.tools.most_common():
        v, why = classify_action(None if tool == "<none>" else tool)
        add(f"| `{tool}` | {n} | `{v}` | {why} |")
    add("")

    add("## What came back from those calls")
    add("")
    for shape, n in sl.shapes.most_common():
        add(f"- `{shape}`: {n}")
    add("")
    tests = sl.shapes.get("test_output", 0)
    t_lo, t_hi = wilson(tests, scored)
    add(f"**Turns acting on test output: {tests}/{scored} = {tests / scored:.1%} "
        f"(CI {t_lo:.1%} – {t_hi:.1%}).**")
    add("")
    add("This is the one that decides whether the customer's own gate is a signal we can "
        "actually reach. 718:718 says the gate outcome *arrives*; this says how often.")

    if sl.unknown_tools:
        add("")
        add("## Tools with no rule")
        add("")
        add("Classified by name, or counted as `unchecked`. Every wrong guess here would "
            "inflate the headline in the direction we want, so the default is pessimistic.")
        add("")
        for tool, n in sl.unknown_tools.most_common(15):
            v, why = classify_action(tool)
            add(f"- `{tool}` ({n}) → `{v}` — {why}")
    return "\n".join(out)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="checkable", description=__doc__.split("\n")[0])
    ap.add_argument("--log", type=Path, required=True, help="a traffic.jsonl recording")
    ap.add_argument("--out", type=Path, default=None)
    ap.add_argument("--all-rows", action="store_true",
                    help="keep the client's blocking retries (they double-count turns)")
    args = ap.parse_args(argv)

    rows = load_rows(args.log, clean_only=not args.all_rows)
    report = render(measure(rows))
    # Write BEFORE printing. The report is the product of this tool and the console is the
    # least reliable thing in the pipeline — this exact run died on `≥` in cp1252 *after*
    # the measurement was complete, over a capture that cost someone twenty minutes.
    if args.out:
        args.out.write_text(report + "\n", encoding="utf-8")
    emit(report)
    if args.out:
        emit(f"\nwritten: {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
