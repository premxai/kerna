"""Earn promotions deliberately, instead of waiting for a week of idle time.

The loop works: the explorer attempts eligible turns locally, `promote.py` turns those
rows into ledger observations, and a cohort with 30 scoreable trials above the bar earns
the right to be served. What it does not do is *hurry*. Exploration runs only when nobody
has made a request for 20 seconds, on turns that happen to arrive — so a pilot waits days
to find out whether anything would ever promote, and learns nothing in the meantime.

This runs the same comparison on purpose. Generate realistic mid-loop turns, ask the
cloud and the local model each for the next action, compare them the way the explorer
does, and write the same rows. **The evidence means exactly what live evidence means**,
which is the whole reason this module generates traffic rather than inventing a second
kind of score. A cohort promoted from here and a cohort promoted from a week of real work
are the same measurement at different speeds.

## Why the turns are generated rather than replayed

Recorded traffic is metadata: counts, block types, tool names, sizes. It is not a
reconstructable request, deliberately (005), so there is nothing to replay. Turns are
therefore built from the customer's own repository the way the replay harness builds tasks —
derived from the tree, never authored, because a hand-written corpus overstated quality
by 3-11x and inverted the model ranking the last time this project trusted one (023).

## What makes a turn eligible

`is_agentic` requires tool definitions **and** a prior tool result: the loop is already
running and nobody is waiting on a first reply (036). So each generated turn carries a
completed `Read` and its output, and asks for the next action. A turn that does not meet
that bar would never be explored in production either, and generating one would measure
a path the product does not take.

## What this cannot do

It cannot tell you the local model is *correct*, only that it chose the same next action
as the cloud on turns like these. Agreement is not correctness — the cloud itself ships a
wrong answer past a test gate about 4% of the time. Promotion authorises *attempting*
locally, with the answer still checked and escalation still free; it has never authorised
believing one.
"""

from __future__ import annotations

import json
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable

from .classify import CLASS_VERSION, class_of
from .ledger import MIN_TRIALS_TO_PROMOTE, Cohort, Ledger
from .oracle import Agreement, compare, extract_action, extract_local_action
from .promote import feed
from .semantic import SEMANTIC_POLICY, compare_semantic

# The tool the generated turn has already completed. Its result is what makes the turn
# agentic, and a Read is the cheapest completed action that carries real file content.
SEED_TOOL = "Read"


@dataclass
class Turn:
    """One generated mid-loop request, with the file it has already read."""

    prompt: str
    tools: list[dict[str, Any]]
    seed_path: str
    seed_content: str

    def payload(self, model: str, *, max_tokens: int = 1024) -> dict[str, Any]:
        """An Anthropic-shaped request that `is_agentic` accepts."""
        return {
            "model": model,
            "max_tokens": max_tokens,
            "tools": self.tools,
            "messages": [
                {"role": "user", "content": self.prompt},
                {"role": "assistant", "content": [{
                    "type": "tool_use", "id": "tu_seed", "name": SEED_TOOL,
                    "input": {"file_path": self.seed_path},
                }]},
                {"role": "user", "content": [{
                    "type": "tool_result", "tool_use_id": "tu_seed",
                    "content": self.seed_content,
                }]},
            ],
        }


@dataclass
class Result:
    """What one qualification run learned, per cohort."""

    rows: list[dict[str, Any]] = field(default_factory=list)
    tally: dict[str, int] = field(default_factory=dict)
    errors: list[str] = field(default_factory=list)


def build_turns(repo: Path, *, limit: int = 40, seed: int = 0,
                max_seed_chars: int = 6_000) -> list[Turn]:
    """Generate mid-loop turns from a repository.

    Each asks a question whose answer is a fact about the tree, and arrives with one file
    already read — so the model's job is to choose the *next* action, which is exactly
    what the explorer compares.
    """
    from ..replay.tasks import generate, python_files, rel
    from ..replay.tools import catalog

    repo = Path(repo).resolve()
    tools = [
        {"name": t["name"], "description": t.get("description", ""),
         "input_schema": t.get("input_schema") or {"type": "object"}}
        for t in catalog("read_only")
    ]

    files = python_files(repo)
    if not files:
        return []

    turns: list[Turn] = []
    for i, task in enumerate(generate(repo, limit=limit, seed=seed)):
        source = files[i % len(files)]
        try:
            body = source.read_text(encoding="utf-8", errors="replace")[:max_seed_chars]
        except OSError:
            continue
        turns.append(Turn(prompt=task.prompt, tools=tools,
                          seed_path=rel(source, repo), seed_content=body))
    return turns


def _row(turn_id: str, cloud, local, *, local_model: str, elapsed_ms: float,
         outcome: str, agreement: Agreement) -> dict[str, Any]:
    """One explore row, in the shape `promote.feed` already understands.

    Written by hand rather than by calling the Explorer, because the Explorer's job is to
    stay off the critical path and this has no critical path to stay off. The *row* is the
    contract between the two, and the tests assert this one satisfies it.
    """
    task_class = class_of(cloud.tool if cloud is not None else None)
    row: dict[str, Any] = {
        "record": "explore",
        "source": "qualify",          # so a cohort's provenance is never guessed at
        "turn": turn_id,
        "task_class": task_class,
        "class_version": CLASS_VERSION,
        "local_model": local_model,
        "outcome": outcome,
        "agreement": agreement.value,
        "semantic_policy": SEMANTIC_POLICY,
        "elapsed_ms": round(elapsed_ms, 1),
        "cloud_tool": cloud.tool if cloud is not None else None,
        "local_tool": local.tool if local is not None else None,
    }
    if cloud is not None and local is not None:
        verdict = compare_semantic(cloud.tool, cloud.args, local.tool, local.args)
        row.update({
            "semantic_equivalent": verdict.equivalent,
            "intent_match": verdict.intent_match,
            "resource_overlap": verdict.resource_overlap,
            "query_match": verdict.query_match,
        })
    return row


def run(turns: list[Turn], *, ask_cloud: Callable[[dict[str, Any]], dict[str, Any]],
        ask_local: Callable[[dict[str, Any]], dict[str, Any] | None],
        local_model: str, cloud_model: str = "claude-opus-5",
        clock: Callable[[], float] = time.monotonic) -> Result:
    """Compare cloud and local on each turn. Never raises for one bad turn.

    A turn that fails on either side is recorded and excluded from the score rather than
    counted as a disagreement: the local model was not fairly asked, and scoring it either
    way would be a lie in one direction.
    """
    out = Result(tally={"turns": 0, "compared": 0, "cloud_failed": 0,
                        "local_failed": 0, "unscorable": 0})

    for i, turn in enumerate(turns):
        out.tally["turns"] += 1
        turn_id = f"qualify-{i:04d}"
        started = clock()

        try:
            cloud_response = ask_cloud(turn.payload(cloud_model))
            cloud = extract_action(cloud_response)
        except Exception as exc:  # noqa: BLE001
            out.tally["cloud_failed"] += 1
            out.errors.append(f"{turn_id}: cloud {type(exc).__name__}: {exc}")
            continue

        if cloud is None:
            # The cloud answered in prose. There is no action to compare against, so
            # this turn cannot measure agreement -- and it is not the local model's fault.
            out.tally["unscorable"] += 1
            out.rows.append(_row(turn_id, None, None, local_model=local_model,
                                 elapsed_ms=(clock() - started) * 1000,
                                 outcome="cloud_no_action",
                                 agreement=Agreement.UNSCORABLE))
            continue

        try:
            local_response = ask_local(turn.payload(local_model))
        except Exception as exc:  # noqa: BLE001
            out.tally["local_failed"] += 1
            out.errors.append(f"{turn_id}: local {type(exc).__name__}: {exc}")
            out.rows.append(_row(turn_id, cloud, None, local_model=local_model,
                                 elapsed_ms=(clock() - started) * 1000,
                                 outcome="did_not_converge",
                                 agreement=Agreement.UNSCORABLE))
            continue

        # The menu the local model was actually offered. The textual transports parse
        # a reply against it, and passing the wrong set would read a valid call as prose
        # -- scoring a transport mismatch as a model failure.
        offered = frozenset(
            str(t.get("name")) for t in turn.tools
            if isinstance(t, dict) and t.get("name")
        )
        local, _wire = (extract_local_action(local_response, allowed_tools=offered)
                        if local_response else (None, "none"))
        if not hasattr(local, "tool"):
            # A string here is an abstention marker, not an action.
            local = None
        if local is None:
            out.rows.append(_row(turn_id, cloud, None, local_model=local_model,
                                 elapsed_ms=(clock() - started) * 1000,
                                 outcome="did_not_converge",
                                 agreement=Agreement.UNSCORABLE))
            continue

        out.tally["compared"] += 1
        out.rows.append(_row(turn_id, cloud, local, local_model=local_model,
                             elapsed_ms=(clock() - started) * 1000,
                             outcome="converged",
                             agreement=compare(local, cloud)))

    return out


def report(ledger: Ledger, *, machine_tier: str, model: str) -> str:
    """Per class: how much evidence, how strong, and whether it promoted."""
    lines = [f"cohorts on {machine_tier} / {model}", ""]
    found = False
    for key, state in sorted(ledger.cohorts.items()):
        cohort = Cohort.from_key(key)
        if cohort.machine_tier != machine_tier or cohort.model != model:
            continue
        found = True
        # The lower bound, never the raw rate. A perfect 5/5 does not clear a 0.25 bar
        # and printing 100% beside it would invite exactly that reading (INV-13).
        bound = state.score.lower_bound
        short = (MIN_TRIALS_TO_PROMOTE - state.trials)
        status = ("PROMOTED" if state.promoted
                  else f"needs {short} more scoreable" if short > 0
                  else f"below the bar ({ledger.theta_econ:.2f})")
        lines.append(
            f"  {cohort.task_class:10} {state.agreements:3}/{state.trials:<3} agree  "
            f"lb {bound:.2f}  {state.unscored:3} unscored  {status}")
    if not found:
        lines.append("  nothing recorded for this machine and model")
    return "\n".join(lines)


def write_rows(rows: list[dict[str, Any]], path: Path) -> None:
    """Append rows to an explore log, so a qualification run and a live run pool."""
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a", encoding="utf-8") as handle:
        for row in rows:
            handle.write(json.dumps(row, ensure_ascii=False) + "\n")


def qualify(repo: Path, *, ask_cloud, ask_local, local_model: str, machine_tier: str,
            ledger: Ledger, limit: int = 40, seed: int = 0,
            explore_log: Path | None = None) -> tuple[Result, dict[str, int]]:
    """Generate, compare, record, and feed the ledger. Returns (result, ledger tally)."""
    turns = build_turns(repo, limit=limit, seed=seed)
    result = run(turns, ask_cloud=ask_cloud, ask_local=ask_local,
                 local_model=local_model)
    if explore_log is not None:
        write_rows(result.rows, explore_log)
    tally = feed(ledger, result.rows, machine_tier=machine_tier)
    return result, tally


# ------------------------------------------------------------------ command line


def make_cloud_asker(upstream: str, *, timeout_s: float = 120.0):
    """Anthropic /v1/messages, with the operator's own key from the environment.

    The key is read from the environment and never stored, logged, or echoed -- the same
    stance the sidecar takes with the header it forwards. An absent key fails loudly here
    rather than producing a run of empty cloud answers that would read as "the cloud had
    no opinion" on every turn.
    """
    import os

    import httpx

    key = os.environ.get("ANTHROPIC_API_KEY")
    if not key:
        raise RuntimeError(
            "ANTHROPIC_API_KEY is not set. The cloud arm is the thing the local model is "
            "being compared against; without it there is nothing to qualify against.")

    base = upstream.rstrip("/")

    def ask(payload: dict[str, Any]) -> dict[str, Any]:
        response = httpx.post(
            f"{base}/messages",
            json=payload,
            headers={"x-api-key": key, "anthropic-version": "2023-06-01"},
            timeout=timeout_s,
        )
        if response.status_code != 200:
            raise RuntimeError(f"cloud {response.status_code}: {response.text[:200]}")
        return response.json()

    return ask


def main(argv: list[str] | None = None) -> int:
    import argparse

    from ..registry.device import profile
    from .identity import resolve_model
    from .interceptor import local_attempt

    parser = argparse.ArgumentParser(
        prog="kerna-observe qualify",
        description="Earn promotions deliberately instead of waiting for idle time.",
    )
    parser.add_argument("--repo", required=True, type=Path,
                        help="the repository turns are generated from")
    parser.add_argument("--local", required=True, metavar="URL",
                        help="local model server, e.g. http://127.0.0.1:8080")
    parser.add_argument("--upstream", default="https://api.anthropic.com/v1")
    parser.add_argument("--cloud-model", default="claude-opus-5")
    parser.add_argument("--limit", type=int, default=40,
                        help="turns to generate; a class needs 30 scoreable to promote")
    parser.add_argument("--seed", type=int, default=0)
    parser.add_argument("--ledger", type=Path, required=True)
    parser.add_argument("--explore-log", type=Path, default=None,
                        help="also append the rows here, so this pools with live evidence")
    parser.add_argument("--local-decoding", default="none")
    parser.add_argument("--local-tool-transport", default="native")
    parser.add_argument("--tool-policy", default="full")
    parser.add_argument("--dry-run", action="store_true",
                        help="generate the turns and stop; calls no model")
    args = parser.parse_args(argv)

    turns = build_turns(args.repo, limit=args.limit, seed=args.seed)
    print(f"generated {len(turns)} turns from {args.repo}")
    if args.dry_run or not turns:
        if turns:
            print(f"  example: {turns[0].prompt[:90]}")
            print(f"  seeded with: {turns[0].seed_path}")
        return 0

    tier = profile().tier.value
    model, source = resolve_model(args.local, None)
    print(f"machine  {tier}")
    print(f"local    {model}  ({source})")
    print(f"cloud    {args.cloud_model}")
    print()

    ledger = Ledger.load(args.ledger)
    result, tally = qualify(
        args.repo,
        ask_cloud=make_cloud_asker(args.upstream),
        ask_local=local_attempt(
            args.local,
            tool_policy=args.tool_policy,
            local_decoding=args.local_decoding,
            local_tool_transport=args.local_tool_transport,
        ),
        local_model=model, machine_tier=tier, ledger=ledger,
        limit=args.limit, seed=args.seed, explore_log=args.explore_log,
    )
    ledger.save(args.ledger)

    print(f"turns {result.tally['turns']}  compared {result.tally['compared']}  "
          f"unscorable {result.tally['unscorable']}  "
          f"cloud failed {result.tally['cloud_failed']}  "
          f"local failed {result.tally['local_failed']}")
    print(f"ledger: {tally['recorded']} recorded, {tally['scored']} scoreable, "
          f"{tally['duplicate']} already counted")
    print()
    print(report(ledger, machine_tier=tier, model=model))

    for line in result.errors[:5]:
        print(f"  !! {line}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
