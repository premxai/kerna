"""What did the agents cost, and what could have run free? (MVP-1)

The buyer-facing report. Everything else in this package is an instrument; this is the
thing a customer actually reads, and the only artefact that has to survive being forwarded
to someone who was not in the room.

Decision 039: the product is one control point, sold on governance and funded by routing.
This is the funding half made legible — **spend on one side, opportunity on the other, and
an explicit account of what has not been measured.**

## The rule this report is built around

**Only turns that were actually attempted and actually agreed may count toward a saving.**
No projection, no extrapolation from a sample to a fleet, no "if this rate held". This
project has been burned twice by numbers that were true of a corpus and false of the
world — a self-authored eval that overstated quality by 3-11x, and a toy project that
understated per-turn cost by 28x. A savings report is exactly the document where that
error is most tempting and most expensive.

## Why cached tokens change the answer

Agent traffic is dominated by cache reads, which bill at a tenth of the input rate. A
report that counts raw tokens will value a cached turn at ten times its real cost and
promise a saving that cannot arrive.

So cost is computed from the four rates the provider actually charges — fresh input,
cache write, cache read, output — and a turn's saving is **its own measured cost**, not
an average. The turns worth routing are the expensive ones, and knowing which those are
is itself part of what the report tells a customer.
"""

from __future__ import annotations

import argparse
import json
import math
from collections import defaultdict
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Iterable

from ..console import emit

# USD per million tokens. Published rates, and they move -- the report prints the table it
# used so a number can always be re-derived rather than trusted.
#
# Cache write is 1.25x input and cache read is 0.10x input; both matter more than the
# headline rate because agent traffic is mostly cache reads.
PRICES: dict[str, tuple[float, float]] = {          # model prefix -> (input, output)
    "claude-opus-4": (5.00, 25.00),
    "claude-opus-5": (5.00, 25.00),
    "claude-sonnet-4": (3.00, 15.00),
    "claude-sonnet-5": (3.00, 15.00),
    "claude-haiku-4": (1.00, 5.00),
    "gpt-4": (2.50, 10.00),
    "gpt-5": (1.25, 10.00),
}
CACHE_WRITE_MULTIPLIER = 1.25
CACHE_READ_MULTIPLIER = 0.10
AGREED = ("same_action_same_args", "same_action")


def price_for(model: str | None) -> tuple[float, float] | None:
    """Rates for a model, or None. An unknown model is reported, never guessed at."""
    if not model:
        return None
    name = model.lower()
    for prefix, rates in sorted(PRICES.items(), key=lambda kv: -len(kv[0])):
        if name.startswith(prefix):
            return rates
    return None


def cost_of(usage: dict[str, Any], model: str | None) -> float | None:
    """One request's cost in USD, or None when the model's rates are unknown."""
    rates = price_for(model)
    if not rates or not isinstance(usage, dict):
        return None
    inp, out = rates
    fresh = float(usage.get("input_tokens") or 0)
    written = float(usage.get("cache_creation_input_tokens") or 0)
    read = float(usage.get("cache_read_input_tokens") or 0)
    produced = float(usage.get("output_tokens") or 0)
    return (
        fresh * inp
        + written * inp * CACHE_WRITE_MULTIPLIER
        + read * inp * CACHE_READ_MULTIPLIER
        + produced * out
    ) / 1_000_000.0


def wilson(successes: int, total: int, z: float = 1.96) -> tuple[float, float]:
    if total == 0:
        return 0.0, 0.0
    p = successes / total
    d = 1 + z * z / total
    centre = (p + z * z / (2 * total)) / d
    half = z * math.sqrt(p * (1 - p) / total + z * z / (4 * total * total)) / d
    return max(0.0, centre - half), min(1.0, centre + half)


@dataclass
class Spend:
    requests: int = 0
    agentic: int = 0
    cost_usd: float = 0.0
    unpriced: int = 0                 # model rates unknown; excluded, never estimated
    by_model: dict[str, float] = field(default_factory=lambda: defaultdict(float))
    by_class: dict[str, float] = field(default_factory=lambda: defaultdict(float))
    # Annotated, so these are per-instance fields rather than shared class attributes.
    # Unannotated they happen to work for ints and would silently break the moment one
    # became a list -- not a bug worth waiting for.
    fresh_in: int = 0
    cache_write: int = 0
    cache_read: int = 0
    out: int = 0


@dataclass
class Opportunity:
    attempted: int = 0
    scored: int = 0
    agreed: int = 0
    at_checkpoint_scored: int = 0
    at_checkpoint_agreed: int = 0
    behaviour_passed: int = 0
    infrastructure: int = 0
    by_class: dict[str, list[int]] = field(default_factory=lambda: defaultdict(lambda: [0, 0]))

    @property
    def measured(self) -> bool:
        return self.scored > 0


def read_traffic(rows: Iterable[dict[str, Any]]) -> Spend:
    sp = Spend()
    for row in rows:
        if row.get("record") != "request":
            continue
        sp.requests += 1
        agentic = bool(row.get("n_tools", 0)) and bool(row.get("has_tool_results"))
        if agentic:
            sp.agentic += 1

        usage = (row.get("response") or {}).get("usage") or {}
        sp.fresh_in += int(usage.get("input_tokens") or 0)
        sp.cache_write += int(usage.get("cache_creation_input_tokens") or 0)
        sp.cache_read += int(usage.get("cache_read_input_tokens") or 0)
        sp.out += int(usage.get("output_tokens") or 0)

        cost = cost_of(usage, row.get("model"))
        if cost is None:
            sp.unpriced += 1
            continue
        sp.cost_usd += cost
        sp.by_model[str(row.get("model"))] += cost
        if agentic:
            sp.by_class[str(row.get("last_tool") or "unknown")] += cost
    return sp


def read_explore(rows: Iterable[dict[str, Any]]) -> Opportunity:
    op = Opportunity()
    for row in rows:
        if row.get("record") != "explore":
            continue
        op.attempted += 1
        if row.get("check_infrastructure"):
            op.infrastructure += 1
            continue
        agreement = row.get("agreement")
        if agreement is None or agreement == "unscorable":
            continue
        op.scored += 1
        agreed = agreement in AGREED
        op.agreed += agreed
        cls = str(row.get("task_class") or "unknown")
        op.by_class[cls][0] += 1
        op.by_class[cls][1] += agreed
        if row.get("edits_since_check", 0) == 0:
            op.at_checkpoint_scored += 1
            op.at_checkpoint_agreed += agreed
        if row.get("check_verdict") == "behaviour_pass":
            op.behaviour_passed += 1
    return op


def render(sp: Spend, op: Opportunity, *, days: float | None = None) -> str:
    out: list[str] = []
    add = out.append
    add("# What your agents cost, and what could have run free")
    add("")

    if not sp.requests:
        add("**No traffic recorded.** Nothing has been measured — this is not a report of "
            "zero spend, it is the absence of a measurement.")
        return "\n".join(out)

    # ---------------------------------------------------------------- spend
    add("## What you spent")
    add("")
    add(f"- requests observed: **{sp.requests:,}** ({sp.agentic:,} agentic)")
    add(f"- **total: ${sp.cost_usd:,.2f}**")
    if days:
        add(f"- per day: **${sp.cost_usd / days:,.2f}**  ·  "
            f"per request: **${sp.cost_usd / max(sp.requests, 1):,.4f}**")
    if sp.unpriced:
        add(f"- **{sp.unpriced} requests excluded** — no published rate for their model. "
            f"Excluded rather than estimated, so this total is a floor.")
    add("")

    billed = sp.fresh_in + sp.cache_write + sp.cache_read + sp.out
    if billed:
        # Token share and cost share are wildly different here, and only one of them is
        # the thing being sold. Showing both is what stops a reader doing the wrong
        # arithmetic in their head.
        rates = price_for(max(sp.by_model, key=sp.by_model.get)) if sp.by_model else None
        add("| tokens | count | share of tokens | share of cost |")
        add("|---|---|---|---|")
        parts = (("fresh input", sp.fresh_in, 1.0), ("cache write", sp.cache_write, CACHE_WRITE_MULTIPLIER),
                 ("cache read", sp.cache_read, CACHE_READ_MULTIPLIER), ("output", sp.out, None))
        weighted = []
        for label, n, mult in parts:
            if rates is None:
                weighted.append((label, n, None))
                continue
            unit = rates[1] if mult is None else rates[0] * mult
            weighted.append((label, n, n * unit))
        total_w = sum(w for _, _, w in weighted if w) or 1.0
        for label, n, w in weighted:
            share = f"{w / total_w:.1%}" if w is not None else "—"
            add(f"| {label} | {n:,} | {n / billed:.1%} | {share} |")
        add("")
        if sp.cache_read > billed * 0.4:
            add(f"**{sp.cache_read / billed:.0%} of your tokens are cache reads**, billed at "
                f"{CACHE_READ_MULTIPLIER:.0%} of the input rate. Your agent already caches "
                f"well, so a report counting raw tokens would overstate what is left to "
                f"save by roughly ten times. The cost column is the one to read.")
            add("")
        if rates and sp.cache_write and sp.out:
            carry = sum(w for label, _, w in weighted if label in ("cache write", "cache read") and w)
            if carry > total_w * 0.5:
                add(f"**{carry / total_w:.0%} of the bill is carrying context, not "
                    f"generating answers.** The expensive part of an agent turn is the "
                    f"conversation it drags behind it. That is worth knowing before "
                    f"anyone proposes a smaller model as the fix — a smaller model "
                    f"generates the cheap part faster.")
                add("")

    if sp.by_model:
        add("| model | cost |")
        add("|---|---|")
        for model, cost in sorted(sp.by_model.items(), key=lambda kv: -kv[1]):
            add(f"| `{model}` | ${cost:,.2f} |")
        add("")

    # ---------------------------------------------------- the opportunity
    add("## What could have run on the laptop")
    add("")
    if not op.measured:
        add("**Not measured.** No local attempt has been scored against a cloud answer "
            "yet, so there is no evidence to report and none is invented here.")
        add("")
        add("Run the sidecar with `--explore` pointed at a local model. It serves nothing "
            "locally and cannot affect a request; it attempts eligible turns on the idle "
            "budget and compares them against the cloud answer you already paid for.")
        return "\n".join(out) + "\n" + _limits(sp, op)

    rate = op.agreed / op.scored
    lo, hi = wilson(op.agreed, op.scored)
    add(f"- turns attempted locally: **{op.attempted:,}**  ·  scored: **{op.scored:,}**")
    add(f"- **agreed with the cloud's own answer: {op.agreed:,} ({rate:.1%}, "
        f"95% CI {lo:.1%}–{hi:.1%})**")
    if op.at_checkpoint_scored:
        c_lo, _ = wilson(op.at_checkpoint_agreed, op.at_checkpoint_scored)
        add(f"- of those at a safe checkpoint: **{op.at_checkpoint_agreed}/"
            f"{op.at_checkpoint_scored}** (lb {c_lo:.1%})")
    if op.behaviour_passed:
        add(f"- and passed your own test suite after the edit was applied: "
            f"**{op.behaviour_passed}**")
    if op.infrastructure:
        add(f"- excluded, our own infrastructure failures: {op.infrastructure}")
    add("")

    # The saving is bounded by the LOWER bound, on the agentic share, and stated as a
    # range. A point estimate here would be the most quoted and least defensible number
    # in the document.
    agentic_cost = sum(sp.by_class.values()) or sp.cost_usd
    add(f"**On the evidence so far, between ${agentic_cost * lo:,.2f} and "
        f"${agentic_cost * hi:,.2f} of ${agentic_cost:,.2f} in agentic spend was work "
        f"the laptop got right.**")
    add("")
    add("That is a *range from a confidence interval*, not a projection. It is what the "
        "measured agreement rate implies for the traffic actually observed — not an "
        "extrapolation to a month, a team, or a fleet.")
    add("")

    if op.by_class:
        add("| task class | scored | agreed | rate |")
        add("|---|---|---|---|")
        for cls, (n, k) in sorted(op.by_class.items(), key=lambda kv: -kv[1][0]):
            add(f"| `{cls}` | {n} | {k} | {k / n:.0%} |")
        add("")

    return "\n".join(out) + "\n" + _limits(sp, op)


def _limits(sp: Spend, op: Opportunity) -> str:
    """The section that makes the rest of the document trustworthy."""
    out = ["## What this does not tell you", ""]
    add = out.append
    add("- **Agreement is not correctness.** These turns were compared against the cloud's "
        "own answer, which ships a wrong answer past a test gate about 4% of the time. "
        "Two models can agree and both be wrong.")
    add("- **Nothing here was served to anyone.** Every answer came from your provider, "
        "unchanged. This is a measurement of what *could* have been, taken at zero risk.")
    if op.measured and op.scored < 100:
        add(f"- **{op.scored} scored turns is a small sample.** The interval above is wide "
            f"for that reason and will narrow with use; treat the lower bound as the "
            f"number, not the midpoint.")
    add("- **A saving requires a machine that can run the model.** Throughput is a cliff, "
        "not a slope: a laptop that fits the model runs it about five times faster than "
        "one that does not, and one with integrated graphics cannot run it at all.")
    add("- **Routing a turn away does not remove all of its cost.** Its context still has "
        "to be carried by the next turn that goes to the cloud, which arrives as a cache "
        "write rather than a cache read. The saving per routed turn is therefore smaller "
        "than that turn's billed cost, and by how much is **not yet measured** — it is the "
        "first thing to check once turns are actually being served locally.")
    add("- **Prices change.** Rates used are printed below so any figure here can be "
        "re-derived rather than trusted.")
    add("")
    add("| | input $/1M | output $/1M |")
    add("|---|---|---|")
    for prefix, (i, o) in sorted(PRICES.items()):
        add(f"| `{prefix}*` | {i:.2f} | {o:.2f} |")
    add("")
    add(f"*Cache writes bill at {CACHE_WRITE_MULTIPLIER:.2f}x input, cache reads at "
        f"{CACHE_READ_MULTIPLIER:.2f}x.*")
    return "\n".join(out)


def load(path: Path) -> list[dict[str, Any]]:
    if not path or not path.is_file():
        return []
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()
            if line.strip()]


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="savings", description=__doc__.split("\n")[0])
    ap.add_argument("--traffic", type=Path, required=True)
    ap.add_argument("--explore", type=Path, default=None)
    ap.add_argument("--days", type=float, default=None,
                    help="observation window, to report a daily rate")
    ap.add_argument("--out", type=Path, default=None)
    args = ap.parse_args(argv)

    if not args.traffic.is_file():
        emit(f"no traffic log at {args.traffic} — nothing has been measured.")
        return 1

    report = render(read_traffic(load(args.traffic)),
                    read_explore(load(args.explore)) if args.explore else Opportunity(),
                    days=args.days)
    if args.out:
        args.out.write_text(report + "\n", encoding="utf-8")
    emit(report)
    if args.out:
        emit(f"\nwritten: {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
