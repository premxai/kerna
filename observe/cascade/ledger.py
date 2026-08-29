"""The Autonomy Ledger — what has earned the right to run locally (Decision 034).

Every other component in this package gathers evidence. This is the one that *decides*
with it, and it is the product: a per-customer map of which work, on which hardware, is
safe to run for free — improving weekly, and impossible for a competitor to start with.

## What a promotion actually authorises

**Electrons, not trust.** A promoted class is one worth *attempting* locally; it is not a
class whose answers are believed. The answer is still checked, and it still escalates on
failure at no cost to the customer.

That is why `THETA_ECON` is **0.25** and not 0.95. Escalation is free, so a class that
converges a quarter of the time still saves money — and below that, attempts are waste
even at zero human cost. Confusing this threshold with a trust threshold would make the
product either useless (bar too high, nothing promotes) or dangerous (bar treated as
safety, the check skipped).

## What counts as an observation

**Only outcomes comparable to what would be served.** Decision 038: an edit made
mid-sequence cannot be validated, because the tree is deliberately broken between edits.
Those attempts are *recorded* — they are useful for understanding the traffic — and they
are **not scored**, because promoting a class on evidence gathered in a state we would
never serve in is exactly the corpus error this project has made twice already.

## Cohorts, and why the classifier version is part of the key

Scores are held per `(task_class × machine_tier × model × class_version)`.

The version is not decoration. `classify.py` stamps it precisely so that a change to what
"edit" *means* starts a fresh cohort rather than silently pooling with months of
observations of something else. A ledger that pools across a definition change is worse
than one with no history at all, because it looks authoritative.

## Hysteresis, or a class flaps forever

Promotion needs the lower bound at or above `THETA_ECON`; demotion happens only below
`THETA_ECON - HYSTERESIS`. Without that gap, a cohort sitting near the threshold promotes
and demotes on alternating observations, and the routing decision becomes a coin flip
that changes with every turn.

## The audit rate never reaches zero

A promoted class keeps sending a decaying fraction of its converged work to the cloud
anyway, to be compared. The fraction decays with evidence because confidence should cost
less over time — and it floors at `MIN_AUDIT_RATE`, because a model, a driver or an OS
update can move behaviour underneath a score that was earned honestly. A ledger with no
audit floor is a ledger that cannot notice it has gone stale.
"""

from __future__ import annotations

import json
import math
import os
import tempfile
import time
from collections import deque
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Iterable

from ..registry.models import EarnedScore

SCHEMA = "m0-ledger-v1"

# Attempting is worth it above this. NOT a trust threshold -- see the module docstring.
THETA_ECON = 0.25
# Demote below THETA_ECON - HYSTERESIS, so a cohort at the boundary does not flap.
HYSTERESIS = 0.05
# Enough observations that a Wilson bound means anything at all.
MIN_TRIALS_TO_PROMOTE = 30
# Drift window: recent outcomes are scored separately from lifetime ones.
DRIFT_WINDOW = 50
# A promoted class is demoted when recent behaviour is *significantly* worse than its
# lifetime record. Two conditions, both required:
#
#   * the observed drop exceeds DRIFT_TOLERANCE, and
#   * the 95% interval on the DIFFERENCE excludes zero.
#
# The first draft compared two Wilson lower bounds directly, which is precisely the
# mistake C0a exists to warn about -- and here it would have been worse than a bad
# report: with a 50-observation window, ordinary sampling noise moves a lower bound by
# far more than two points, so healthy cohorts would have demoted themselves at random
# and the routing decision would have looked haunted.
DRIFT_TOLERANCE = 0.02
# The audit rate decays with evidence but never below this.
MIN_AUDIT_RATE = 0.02
START_AUDIT_RATE = 0.25


def wilson_lower_bound(successes: int, total: int, z: float = 1.96) -> float:
    if total == 0:
        return 0.0
    p = successes / total
    d = 1 + z * z / total
    centre = (p + z * z / (2 * total)) / d
    half = z * math.sqrt(p * (1 - p) / total + z * z / (4 * total * total)) / d
    return max(0.0, centre - half)


def _significantly_worse(s1: int, n1: int, s2: int, n2: int, z: float = 1.96) -> bool:
    """Is rate 1 significantly below rate 2? Agresti-Caffo on the difference.

    The interval on the difference, never a comparison of two bounds. Agresti-Caffo
    rather than Wald because Wald's standard error is exactly zero at p=0 and p=1, which
    are the values a 50-observation window lands on most often when something has just
    broken -- and a zero-width interval would call every such window significant.
    """
    if not n1 or not n2:
        return False
    p1 = (s1 + 1) / (n1 + 2)
    p2 = (s2 + 1) / (n2 + 2)
    se = math.sqrt(p1 * (1 - p1) / (n1 + 2) + p2 * (1 - p2) / (n2 + 2))
    upper = (s1 / n1) - (s2 / n2) + z * se
    return upper < 0.0            # the whole interval sits below zero


@dataclass(frozen=True)
class Cohort:
    """One scoreable population. The version is part of the identity, not a label."""

    task_class: str
    machine_tier: str
    model: str
    class_version: str = "1"

    def key(self) -> str:
        return f"{self.task_class}|{self.machine_tier}|{self.model}|v{self.class_version}"

    @classmethod
    def from_key(cls, key: str) -> Cohort:
        # rsplit, not split: the task class comes first and is the only field a
        # classifier could put a delimiter into. Splitting from the left turned
        # `a|weird` into two fields and silently produced a different cohort.
        task_class, tier, model, version = key.rsplit("|", 3)
        return cls(task_class, tier, model, version.lstrip("v"))


@dataclass
class CohortState:
    trials: int = 0
    agreements: int = 0
    recent: deque = field(default_factory=lambda: deque(maxlen=DRIFT_WINDOW))
    promoted: bool = False
    promoted_at: float | None = None
    demoted_at: float | None = None
    demoted_reason: str | None = None
    audits: int = 0
    unscored: int = 0            # observed but not comparable to what we would serve

    @property
    def score(self) -> EarnedScore:
        return EarnedScore(task_class="", agreements=self.agreements, trials=self.trials)

    @property
    def lower_bound(self) -> float:
        return wilson_lower_bound(self.agreements, self.trials)

    @property
    def recent_lower_bound(self) -> float:
        if not self.recent:
            return 0.0
        return wilson_lower_bound(sum(self.recent), len(self.recent))

    def as_dict(self) -> dict[str, Any]:
        return {
            "trials": self.trials, "agreements": self.agreements,
            "recent": list(self.recent), "promoted": self.promoted,
            "promoted_at": self.promoted_at, "demoted_at": self.demoted_at,
            "demoted_reason": self.demoted_reason, "audits": self.audits,
            "unscored": self.unscored,
        }

    @classmethod
    def from_dict(cls, row: dict[str, Any]) -> CohortState:
        state = cls(
            trials=int(row.get("trials") or 0),
            agreements=int(row.get("agreements") or 0),
            promoted=bool(row.get("promoted")),
            promoted_at=row.get("promoted_at"),
            demoted_at=row.get("demoted_at"),
            demoted_reason=row.get("demoted_reason"),
            audits=int(row.get("audits") or 0),
            unscored=int(row.get("unscored") or 0),
        )
        state.recent.extend(int(bool(x)) for x in (row.get("recent") or []))
        return state


@dataclass
class Ledger:
    """Accumulates evidence and decides what may be attempted locally."""

    path: Path | None = None
    theta_econ: float = THETA_ECON
    cohorts: dict[str, CohortState] = field(default_factory=dict)
    clock: Any = time.time
    # Turn ids already recorded. A restart that re-reads its own explore log would
    # otherwise count every observation a second time and inflate the evidence behind a
    # promotion -- the one number that must never be inflated, because it is what
    # authorises serving a local answer to a customer.
    seen_turns: set[str] = field(default_factory=set)

    # ------------------------------------------------------------- observing

    def observe(self, cohort: Cohort, agreed: bool, *, scoreable: bool = True,
                audited: bool = False) -> None:
        """Record one outcome.

        `scoreable=False` records an attempt that cannot count toward promotion — a
        mid-sequence edit, an unscorable comparison, a turn whose check could not run.
        It is kept because the *count* is informative and dropping it would make the
        ledger look better-evidenced than it is.
        """
        state = self.cohorts.setdefault(cohort.key(), CohortState())
        if audited:
            state.audits += 1
        if not scoreable:
            state.unscored += 1
            return

        state.trials += 1
        state.agreements += int(bool(agreed))
        state.recent.append(int(bool(agreed)))
        self._reconsider(cohort, state)

    def _reconsider(self, cohort: Cohort, state: CohortState) -> None:
        now = self.clock()
        if not state.promoted:
            if state.trials < MIN_TRIALS_TO_PROMOTE or state.lower_bound < self.theta_econ:
                return
            # A demoted cohort must not re-promote on its lifetime score alone. After a
            # long healthy history the lifetime bound stays high for thousands of
            # observations, so the very next outcome would re-promote a class that just
            # collapsed -- and it would demote again on the one after that. Demotion
            # without this check is decoration.
            if self._recent_is_worse(state):
                return
            state.promoted = True
            state.promoted_at = now
            state.demoted_reason = None
            return

        # Demotion has two independent triggers and they answer different questions.
        floor = self.theta_econ - HYSTERESIS
        if state.lower_bound < floor:
            self._demote(state, now, f"lifetime lower bound {state.lower_bound:.3f} "
                                     f"below floor {floor:.3f}")
            return
        if self._recent_is_worse(state):
            # Behaviour moved under a score that was earned honestly -- a model swap, a
            # driver update, a change in the kind of work being sent.
            recent_rate = sum(state.recent) / len(state.recent)
            lifetime_rate = state.agreements / state.trials
            self._demote(state, now,
                         f"recent {recent_rate:.1%} vs lifetime {lifetime_rate:.1%}, "
                         f"a significant drop")

    def _recent_is_worse(self, state: CohortState) -> bool:
        """Has the recent window fallen significantly below the lifetime record?

        Used in both directions -- to demote a promoted cohort, and to refuse to
        re-promote one until the recent window recovers. Symmetry is the point: a rule
        that only fires on the way down lets a collapsed class oscillate.
        """
        if len(state.recent) < DRIFT_WINDOW or not state.trials:
            return False
        recent_ok, recent_n = sum(state.recent), len(state.recent)
        drop = (state.agreements / state.trials) - (recent_ok / recent_n)
        return drop > DRIFT_TOLERANCE and _significantly_worse(
            recent_ok, recent_n, state.agreements, state.trials)

    def _demote(self, state: CohortState, now: float, reason: str) -> None:
        state.promoted = False
        state.demoted_at = now
        state.demoted_reason = reason

    # -------------------------------------------------------------- deciding

    def state_of(self, cohort: Cohort) -> CohortState:
        return self.cohorts.get(cohort.key(), CohortState())

    def may_attempt(self, cohort: Cohort) -> bool:
        return self.state_of(cohort).promoted

    def promoted_classes(self, *, machine_tier: str, model: str,
                         class_version: str = "1") -> frozenset[str]:
        """What the dispatcher may attempt on *this* machine with *this* model.

        Scoped deliberately. A class earned on a 24 GB workstation says nothing about an
        8 GB laptop, and pooling them would promote a class onto hardware that has never
        run it.
        """
        out = set()
        for key, state in self.cohorts.items():
            if not state.promoted:
                continue
            cohort = Cohort.from_key(key)
            if (cohort.machine_tier == machine_tier and cohort.model == model
                    and cohort.class_version == class_version):
                out.add(cohort.task_class)
        return frozenset(out)

    def audit_rate(self, cohort: Cohort) -> float:
        """What fraction of this cohort's converged work should still go to the cloud.

        Decays with evidence and floors above zero. The floor is the point: a score
        earned honestly can be invalidated by a model, driver or OS update, and a ledger
        that stops looking cannot notice.
        """
        state = self.state_of(cohort)
        if not state.promoted:
            return 1.0                      # unpromoted work is all evidence anyway
        decayed = START_AUDIT_RATE / math.sqrt(max(state.trials, 1))
        return max(MIN_AUDIT_RATE, min(START_AUDIT_RATE, decayed))

    # ------------------------------------------------------------ persistence

    def save(self, path: Path | None = None) -> None:
        """Atomic write. A ledger half-written by a crash is worse than none.

        The file is the only durable record of months of observation; a torn write at
        the wrong moment would silently reset a customer's earned autonomy to zero and
        look exactly like a fresh install.
        """
        target = path or self.path
        if target is None:
            raise ValueError("no path to save to")
        target.parent.mkdir(parents=True, exist_ok=True)
        payload = {
            "schema": SCHEMA,
            "written": time.strftime("%Y-%m-%dT%H:%M:%S"),
            "theta_econ": self.theta_econ,
            "cohorts": {k: v.as_dict() for k, v in self.cohorts.items()},
            # Sorted so two saves of the same state are byte-identical and a diff of the
            # ledger shows what changed rather than how a set happened to iterate.
            "seen_turns": sorted(self.seen_turns),
        }
        handle, tmp = tempfile.mkstemp(dir=str(target.parent), suffix=".tmp")
        try:
            with os.fdopen(handle, "w", encoding="utf-8") as fh:
                json.dump(payload, fh, indent=2)
                fh.flush()
                os.fsync(fh.fileno())
            os.replace(tmp, target)
        except BaseException:
            try:
                os.unlink(tmp)
            except OSError:
                pass
            raise

    @classmethod
    def load(cls, path: Path) -> Ledger:
        """Read a ledger, or return an empty one. A corrupt file never crashes a request.

        An unreadable ledger means no class is promoted, which is the safe direction:
        everything goes to the cloud, exactly as it does on a fresh install.
        """
        ledger = cls(path=path)
        if not path.is_file():
            return ledger
        try:
            payload = json.loads(path.read_text(encoding="utf-8"))
            if payload.get("schema") != SCHEMA:
                return ledger                 # a different schema is not our history
            ledger.theta_econ = float(payload.get("theta_econ") or THETA_ECON)
            for key, row in (payload.get("cohorts") or {}).items():
                if isinstance(row, dict) and "|" in key:
                    ledger.cohorts[key] = CohortState.from_dict(row)
            seen = payload.get("seen_turns")
            if isinstance(seen, list):
                ledger.seen_turns = {str(t) for t in seen if t}
        except Exception:  # noqa: BLE001
            return cls(path=path)
        return ledger

    # ---------------------------------------------------------------- report

    def render(self) -> str:
        lines = ["# Autonomy Ledger", ""]
        if not self.cohorts:
            lines.append("**Nothing observed.** No class has been attempted, so none is "
                         "promoted and every request goes to the cloud.")
            return "\n".join(lines)

        lines.append(f"Attempting is authorised at a lower bound of **{self.theta_econ:.0%}** "
                     f"— that is a threshold for spending electrons, not for trusting an "
                     f"answer. Every local answer is still checked and still escalates.")
        lines.append("")
        lines.append("| cohort | trials | agreed | lower bound | state | audit rate |")
        lines.append("|---|---|---|---|---|---|")
        for key in sorted(self.cohorts):
            state = self.cohorts[key]
            cohort = Cohort.from_key(key)
            if state.promoted:
                mark = "**promoted**"
            elif state.demoted_reason:
                mark = "demoted"
            elif state.trials < MIN_TRIALS_TO_PROMOTE:
                mark = f"learning ({MIN_TRIALS_TO_PROMOTE - state.trials} more)"
            else:
                mark = "below bar"
            lines.append(
                f"| `{cohort.task_class}` · {cohort.machine_tier} · {cohort.model} "
                f"| {state.trials} | {state.agreements} | {state.lower_bound:.1%} "
                f"| {mark} | {self.audit_rate(cohort):.0%} |")
        lines.append("")

        demoted = [(k, s) for k, s in sorted(self.cohorts.items()) if s.demoted_reason]
        if demoted:
            lines.append("## Demotions")
            lines.append("")
            for key, state in demoted:
                lines.append(f"- `{key}` — {state.demoted_reason}")
            lines.append("")

        unscored = sum(s.unscored for s in self.cohorts.values())
        if unscored:
            plural = "attempt was" if unscored == 1 else "attempts were"
            lines.append(f"**{unscored} {plural} recorded but not scored** — made in "
                         f"a state we would never serve in, most often an edit partway "
                         f"through a sequence. Counting them would make this ledger look "
                         f"better evidenced than it is.")
        return "\n".join(lines)


def observations_from(rows: Iterable[dict[str, Any]], *, machine_tier: str,
                      model: str) -> Iterable[tuple[Cohort, bool, bool]]:
    """Turn exploration rows into ledger observations.

    Yields `(cohort, agreed, scoreable)`. A row is scoreable only when the comparison
    happened *and* the turn was in a state we would serve in — Decision 038.
    """
    for row in rows:
        if row.get("record") != "explore":
            continue
        cohort = Cohort(task_class=str(row.get("task_class") or "unknown"),
                        machine_tier=machine_tier, model=model,
                        class_version=str(row.get("class_version") or "1"))
        agreement = row.get("agreement")
        comparable = agreement in ("same_action_same_args", "same_action",
                                   "different_action")
        at_checkpoint = int(row.get("edits_since_check") or 0) == 0
        scoreable = bool(comparable and at_checkpoint
                         and not row.get("check_infrastructure"))
        yield cohort, agreement in ("same_action_same_args", "same_action"), scoreable
