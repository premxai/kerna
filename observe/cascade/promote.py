"""The missing half of the loop: turn EXPLORE rows into earned autonomy.

Every piece of the cascade existed except the join. The explorer wrote rows, the ledger
knew how to promote a cohort, and **nothing connected them** — the ledger was written to
by its own tests and by nothing else. So `promoted_classes` was permanently empty, which
means no request could ever route locally no matter what the fairness gate said.

That is worth stating precisely, because it was easy to misread the symptom: routing was
not off because `LOCAL_ROUTING_ENABLED` is False. It was off because **nothing had ever
been promoted and nothing could be**. Opening the gate on its own would have changed
exactly nothing.

## What counts as an agreement

`semantic_equivalent` under the frozen `semantic-v1` rule, when present, falling back to
the strict action comparison. Both are recorded on the row, so this module chooses
between two existing verdicts rather than inventing a third — a scorer with its own
opinion is a knob that makes our own number go up (`semantic.py` says the same thing
about itself).

## What is deliberately not scoreable

Promotion evidence must be comparable to what would actually be served, so these are
recorded and excluded from the score rather than dropped:

    unscorable comparisons          nothing to compare against
    the local model did not converge  no action was produced
    lossy dialect translation        the shadow saw a different request
    the cloud's tool was not offered  the local model never had that button

Dropping them entirely would make the ledger look better-evidenced than it is; counting
them as disagreements would understate a model that was never given the chance.

## Replay is idempotent

Rows carry a turn id and the ledger records which it has already seen, so feeding the
same log twice does not double a cohort's trials. Without that, a restart that re-read
its own log would inflate the evidence behind a promotion — which is the one number that
must never be inflated, because it is what authorises serving.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any, Iterable

from .classify import CLASS_VERSION
from .ledger import Cohort, Ledger

# Outcomes that mean "the local model was never fairly asked". Recorded, never scored.
NOT_A_FAIR_TRIAL = frozenset({
    "action_space_ineligible",
    "translation_ineligible",
    "context_ineligible",
    "did_not_converge",
})


def _agreed(row: dict[str, Any]) -> bool:
    """Whether this row counts as the local model choosing the same thing.

    Prefers the frozen `semantic-v1` interpretation when the row carries one, because
    exact action equality answers a question nobody asked -- `Bash("grep -rn x")` and
    `Grep(pattern="x")` are the same decision. Falls back to strict equality when the
    semantic verdict is absent, which is what old rows have.
    """
    semantic = row.get("semantic_equivalent")
    if isinstance(semantic, bool):
        return semantic
    return row.get("agreement") in ("same_action_same_args", "same_action")


def scoreable(row: dict[str, Any]) -> bool:
    """Whether this row may count toward promotion."""
    if row.get("outcome") in NOT_A_FAIR_TRIAL:
        return False
    if row.get("agreement") == "unscorable":
        return False
    # An edit made mid-sequence cannot be validated: the tree is deliberately broken
    # between edits, so the check would fail for a reason that has nothing to do with
    # the model (Decision 038).
    if row.get("at_checkpoint") is False:
        return False
    return True


def cohort_of(row: dict[str, Any], *, machine_tier: str) -> Cohort | None:
    """The cohort this row belongs to, or None when it cannot be attributed.

    A row missing its class or its model is unattributable, and guessing would pool it
    into a cohort it was not measured in -- the exact error `class_version` exists to
    prevent.
    """
    task_class = row.get("task_class")
    model = row.get("local_model")
    if not task_class or not model:
        return None
    return Cohort(
        task_class=str(task_class),
        machine_tier=machine_tier,
        model=str(model),
        class_version=str(row.get("class_version") or CLASS_VERSION),
    )


def feed(ledger: Ledger, rows: Iterable[dict[str, Any]], *,
         machine_tier: str) -> dict[str, int]:
    """Record explore rows into the ledger. Returns a small tally, for the operator.

    Skips rows already recorded, so a log may be fed repeatedly without inflating the
    evidence behind a promotion.
    """
    tally = {"seen": 0, "recorded": 0, "scored": 0, "duplicate": 0, "unattributable": 0}

    for row in rows:
        if row.get("record") != "explore":
            continue
        tally["seen"] += 1

        turn = row.get("turn")
        if turn and turn in ledger.seen_turns:
            tally["duplicate"] += 1
            continue

        cohort = cohort_of(row, machine_tier=machine_tier)
        if cohort is None:
            tally["unattributable"] += 1
            continue

        can_score = scoreable(row)
        ledger.observe(cohort, _agreed(row) if can_score else False,
                       scoreable=can_score)
        if turn:
            ledger.seen_turns.add(str(turn))
        tally["recorded"] += 1
        if can_score:
            tally["scored"] += 1

    return tally


def read_rows(path: Path) -> list[dict[str, Any]]:
    """Explore rows from a JSONL log, skipping anything unparsable."""
    out: list[dict[str, Any]] = []
    if not Path(path).is_file():
        return out
    for line in Path(path).read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            row = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(row, dict):
            out.append(row)
    return out


def promoted_for(ledger: Ledger, *, machine_tier: str, model: str) -> frozenset[str]:
    """What may be served locally on this machine, with this model, right now."""
    return ledger.promoted_classes(
        machine_tier=machine_tier, model=model, class_version=CLASS_VERSION)
