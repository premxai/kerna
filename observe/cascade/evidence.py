"""Turn exploration rows into a routing decision (P1.2-D).

The explorer writes one row per local attempt. This reads them and answers the questions
that decide whether the serving MVP is worth building — with Wilson bounds on every rate
(INV-13) and the interval on the **difference** whenever two are compared, because
comparing two lower bounds is what would have read C0a as a clear cloud win and shut the
project.

## The question P1.1 forced, and the reason this file leads with it

Decision 038: checkability is a position in a sequence, not a property of a turn. The
measured trace was `Edit ×8 → Bash(tests)`, so seven of those eight edits had no
meaningful check available — mid-sequence the tree is deliberately broken and the suite
fails for reasons unrelated to the turn under test.

That is a *mechanism*, argued from one capture. The number that confirms or kills it is
**agreement at a checkpoint against agreement mid-sequence.** If they are the same, 038 is
wrong and cheap routing is available on every eligible turn. If they diverge, routing has
to detect checkpoints and "attempt every eligible turn" was never the right design.

This split is computable from rows the explorer already writes, needs no sandbox and no
check implementation, and it is the highest-value thing in P1.2-D. It runs first.

## What this file will not do

**It will not report an arm it has no evidence for.** The three-arm check comparison —
form against behaviour against nothing — needs a check implementation and a sandbox that
do not exist yet. An empty arm is reported as *not measured*, never as zero, because
`checkable.py` was asked the same question a day earlier and reporting 0% would have been
a finding rather than an absence.

**It will not call agreement an accuracy.** The cloud ships a wrong answer past a test
gate 3.8% of the time; agreement with it is a proxy, and the ε-audits in 034 exist because
it is one.
"""

from __future__ import annotations

import argparse
import json
import math
from collections import Counter
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Iterable

from ..console import emit

AGREED = ("same_action_same_args", "same_action")
GATE_BEHAVIOUR_PRECISION_LB = 0.90

# Below this many scored attempts per side, no comparison is claimed at all.
#
# Normal approximations are outside their range down here, and they fail in the
# expensive direction: three-for-three against zero-for-three produces an interval
# excluding zero under both Wald and Agresti-Caffo, while Fisher's exact test on the same
# table gives p = 0.10. So the interval would announce that position matters, on six
# observations, in a report written to settle exactly that question.
MIN_SCORED_TO_COMPARE = 10


def wilson(successes: int, total: int, z: float = 1.96) -> tuple[float, float]:
    if total == 0:
        return 0.0, 0.0
    p = successes / total
    d = 1 + z * z / total
    centre = (p + z * z / (2 * total)) / d
    half = z * math.sqrt(p * (1 - p) / total + z * z / (4 * total * total)) / d
    return max(0.0, centre - half), min(1.0, centre + half)


def diff_interval(s1: int, n1: int, s2: int, n2: int,
                  z: float = 1.96) -> tuple[float, float] | None:
    """95% CI on p1 - p2 (Agresti-Caffo), or None when either side has no observations.

    The textbook Wald interval is wrong here in a way that matters, and a test caught it:
    at p = 0 or p = 1 its standard error is **exactly zero**, so three-for-three against
    zero-for-three reports a difference of 100% with a zero-width interval — perfect
    certainty from six observations. That is the same error INV-13 exists to prevent, and
    it appears at precisely the boundaries small evidence lands on.

    Agresti-Caffo adds one success and one failure to each arm before computing. The
    interval stays sane at the boundaries, and on large samples it converges to Wald.
    """
    if not n1 or not n2:
        return None
    p1 = (s1 + 1) / (n1 + 2)
    p2 = (s2 + 1) / (n2 + 2)
    se = math.sqrt(p1 * (1 - p1) / (n1 + 2) + p2 * (1 - p2) / (n2 + 2))
    # The point estimate stays the observed difference; only the interval is adjusted, so
    # the headline number is still the thing that was actually seen.
    delta = (s1 / n1) - (s2 / n2)
    return delta - z * se, delta + z * se


@dataclass
class Arm:
    """One population of attempts, and how often they agreed with the cloud."""

    name: str
    scored: int = 0            # attempts where a comparison was possible
    agreed: int = 0
    unscorable: int = 0
    did_not_converge: int = 0
    failed: int = 0

    @property
    def measured(self) -> bool:
        return self.scored > 0

    @property
    def rate(self) -> float:
        return self.agreed / self.scored if self.scored else 0.0

    @property
    def bounds(self) -> tuple[float, float]:
        return wilson(self.agreed, self.scored)

    def observe(self, row: dict[str, Any]) -> None:
        outcome = row.get("outcome")
        if outcome == "infrastructure_failure":
            self.failed += 1
            return
        if outcome == "did_not_converge":
            self.did_not_converge += 1
            return
        agreement = row.get("agreement")
        if agreement is None or agreement == "unscorable":
            self.unscorable += 1
            return
        self.scored += 1
        if agreement in AGREED:
            self.agreed += 1


@dataclass
class Evidence:
    rows: int = 0
    at_checkpoint: Arm = field(default_factory=lambda: Arm("at a checkpoint"))
    mid_sequence: Arm = field(default_factory=lambda: Arm("mid-sequence"))
    by_class: dict[str, Arm] = field(default_factory=dict)
    agreement: Counter = field(default_factory=Counter)
    checks_seen: Counter = field(default_factory=Counter)
    check_infrastructure: int = 0
    # The three-arm comparison. `no_check` is the null: accept everything, and its
    # precision is the base agreement rate. An arm that cannot beat it is not a cheaper
    # route to the same place -- it is not a route.
    form_accepted: Arm = field(default_factory=lambda: Arm("form check accepted"))
    behaviour_accepted: Arm = field(default_factory=lambda: Arm("behaviour check accepted"))
    no_check: Arm = field(default_factory=lambda: Arm("no check (accept everything)"))


def read(rows: Iterable[dict[str, Any]]) -> Evidence:
    ev = Evidence()
    for row in rows:
        if row.get("record") != "explore":
            continue
        ev.rows += 1
        ev.agreement[row.get("agreement") or row.get("outcome") or "?"] += 1
        if row.get("check_verdict"):
            ev.checks_seen[row["check_verdict"]] += 1

        arm = ev.at_checkpoint if row.get("edits_since_check", 0) == 0 else ev.mid_sequence
        arm.observe(row)

        cls = str(row.get("task_class") or "unknown")
        ev.by_class.setdefault(cls, Arm(cls)).observe(row)

        # An infrastructure failure is excluded from every arm rather than counted as a
        # check that rejected the answer. Ours must never read as the model's.
        if row.get("check_infrastructure"):
            ev.check_infrastructure += 1
            continue
        ev.no_check.observe(row)
        if row.get("form_verdict") == "form_pass":
            ev.form_accepted.observe(row)
        if row.get("check_verdict") == "behaviour_pass":
            ev.behaviour_accepted.observe(row)
    return ev


def _arm_line(arm: Arm) -> str:
    if not arm.measured:
        return f"| {arm.name} | — | — | **not measured** | — |"
    lo, hi = arm.bounds
    return (f"| {arm.name} | {arm.scored} | {arm.agreed} | "
            f"**{arm.rate:.1%}** | {lo:.1%} – {hi:.1%} |")


def render(ev: Evidence) -> str:
    out: list[str] = []
    add = out.append
    add("# P1.2-D — what the exploration evidence says")
    add("")

    if not ev.rows:
        add("**No exploration rows. Nothing has been measured.**")
        add("")
        add("This is not a result. Run the sidecar with an explorer attached and a local "
            "model behind it, then read this again.")
        return "\n".join(out)

    add(f"Attempts recorded: **{ev.rows}**")
    add("")

    # ---- the question 038 raised, first because it can invalidate the rest
    add("## Agreement by position in the sequence")
    add("")
    add("Decision 038 says a check applied mid-sequence answers a question nobody asked, "
        "because the tree is deliberately broken between edits. If these two rates match, "
        "038 is wrong and routing is far cheaper than the plan assumes.")
    add("")
    add("| position | scored | agreed | agreement | 95% CI |")
    add("|---|---|---|---|---|")
    add(_arm_line(ev.at_checkpoint))
    add(_arm_line(ev.mid_sequence))
    add("")

    thin = min(ev.at_checkpoint.scored, ev.mid_sequence.scored) < MIN_SCORED_TO_COMPARE
    interval = diff_interval(ev.at_checkpoint.agreed, ev.at_checkpoint.scored,
                             ev.mid_sequence.agreed, ev.mid_sequence.scored)
    if interval is None:
        add("**The difference cannot be computed** — one position has no scored attempts. "
            "The evidence does not yet speak to 038 either way.")
    elif thin:
        add(f"**Too few observations to compare** — fewer than {MIN_SCORED_TO_COMPARE} "
            f"scored attempts on one side. **This evidence cannot separate them**, and no "
            f"interval is quoted: down here the normal approximation announces "
            f"significance that an exact test does not support, which is the one error "
            f"this report exists to avoid. 038 **remains an argument from one capture**.")
    else:
        lo, hi = interval
        delta = ev.at_checkpoint.rate - ev.mid_sequence.rate
        add(f"**Checkpoint minus mid-sequence: {delta:+.1%}, 95% CI [{lo:+.1%}, {hi:+.1%}].**")
        if lo > 0:
            add("The interval excludes zero: position matters, and 038 holds. Routing must "
                "detect checkpoints.")
        elif hi < 0:
            add("The interval excludes zero **in the opposite direction** — mid-sequence "
                "attempts agree *more*. 038 is not merely unsupported, it is backwards, "
                "and the reason needs finding before anything is built on it.")
        else:
            add("The interval includes zero: **this evidence cannot separate them.** 038 "
                "remains an argument from one capture, and the checkpoint gate is currently "
                "costing coverage for a reason nobody has confirmed.")
    add("")

    # ---- per class
    add("## Agreement by task class")
    add("")
    add("| class | scored | agreed | agreement | 95% CI |")
    add("|---|---|---|---|---|")
    for name in sorted(ev.by_class):
        add(_arm_line(ev.by_class[name]))
    add("")

    # ---- what happened to attempts that never reached a comparison
    add("## Attempts that produced no verdict")
    add("")
    total = ev.at_checkpoint, ev.mid_sequence
    add(f"- did not converge: {sum(a.did_not_converge for a in total)}")
    add(f"- unscorable (one side answered in prose): {sum(a.unscorable for a in total)}")
    add(f"- infrastructure failures: {sum(a.failed for a in total)}")
    add("")
    add("Non-convergence is the cascade working, not a fault. Infrastructure failures are "
        "ours and a rising count means the local path is broken, not that the model is bad.")
    add("")

    # ---- the three-arm check comparison, if it exists yet
    add("## The check arms")
    add("")
    if ev.check_infrastructure:
        add(f"> **{ev.check_infrastructure} of {ev.rows} checks failed for infrastructure "
            f"reasons** — a missing sandbox, an unrunnable suite, a timeout in our own "
            f"code. These are ours, not the model's, and they are excluded from every "
            f"rate below rather than counted as failed checks. A high count here means "
            f"the environment is broken, not that the model cannot write code.")
        add("")

    if not ev.checks_seen:
        add("**Not measured.** No row carries a `check_verdict`, so the form-against-"
            "behaviour comparison has not been run. It needs a check implementation and a "
            "sandbox to apply edits in, and neither exists yet.")
        add("")
        add(f"When it does, the gate is: **behaviour-check acceptance precision ≥ "
            f"{GATE_BEHAVIOUR_PRECISION_LB:.0%} at the lower bound.** Below that, Decision "
            "029 has no production instantiation and the serving product does not exist, "
            "however good the benchmark numbers were.")
    else:
        add("Acceptance precision: of the attempts a check **accepted**, how many agreed "
            "with the cloud. This is the number Decision 029 lives or dies on, and the "
            "form arm is expected to fail it — run anyway, because a boundary claimed "
            "without its control is an argument rather than a finding.")
        add("")
        add("| arm | accepted | agreed | precision | 95% CI |")
        add("|---|---|---|---|---|")
        add(_arm_line(ev.form_accepted))
        add(_arm_line(ev.behaviour_accepted))
        add(_arm_line(ev.no_check))
        add("")

        if ev.behaviour_accepted.measured:
            lo, _ = ev.behaviour_accepted.bounds
            passes = lo >= GATE_BEHAVIOUR_PRECISION_LB
            add(f"**Gate — behaviour-check acceptance precision ≥ "
                f"{GATE_BEHAVIOUR_PRECISION_LB:.0%} at the lower bound: "
                f"{'PASSES' if passes else 'FAILS'}.**  "
                f"({ev.behaviour_accepted.agreed}/{ev.behaviour_accepted.scored}, "
                f"lb {lo:.1%})")
            if not passes:
                add("")
                add("Decision 029 has no production instantiation: the strongest check "
                    "available does not discriminate on real traffic, however good the "
                    "benchmark numbers were.")
        else:
            add("**Gate not evaluated** — no attempt was accepted by the behaviour check.")

        add("")
        add("| verdict | n |")
        add("|---|---|")
        for verdict, n in ev.checks_seen.most_common():
            add(f"| `{verdict}` | {n} |")
    add("")
    add("---")
    add("")
    add("*Every rate above is an **agreement** rate against the cloud's own answer, which "
        "ships a wrong answer past a test gate 3.8% of the time. It is a proxy for "
        "correctness and not a measurement of it — the ε-audits in Decision 034 exist "
        "because of exactly this gap.*")
    return "\n".join(out)


def load(path: Path) -> list[dict[str, Any]]:
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()
            if line.strip()]


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="evidence", description=__doc__.split("\n")[0])
    ap.add_argument("--log", type=Path, required=True, help="an exploration log")
    ap.add_argument("--out", type=Path, default=None)
    args = ap.parse_args(argv)

    if not args.log.exists():
        emit(f"no exploration log at {args.log} — nothing has been measured.")
        return 1

    report = render(read(load(args.log)))
    if args.out:                       # written first: the console is the unreliable part
        args.out.write_text(report + "\n", encoding="utf-8")
    emit(report)
    if args.out:
        emit(f"\nwritten: {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
