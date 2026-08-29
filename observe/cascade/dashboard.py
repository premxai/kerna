"""One page that answers what the agents cost, what we stopped, and what could run free.

The buyer-facing surface of MVP-1. `savings.py` writes the report someone forwards;
this is the thing someone *watches*, and the difference in job shows up in the design —
summary before detail, state encoded as shape and colour rather than only as a number, and
the panel needing attention readable at arm's length.

It joins four sources on the turn id:

  * `traffic.jsonl` — spend, tokens, the cache split
  * `explore.jsonl` — what the local model would have done, and what policy blocked
  * `ledger.json`   — what has earned the right to be attempted locally
  * and the gate rows inside the exploration log

## Why there is no linked webfont

This file is generated on a customer's machine and opened there, often on a laptop with no
network or on a network that will not reach a font CDN. A page whose typography silently
collapses in front of a VP is worse than one that never reached for the font. The
personality is carried by treatment — a strict scale, tabular figures everywhere, tight
uppercase micro-labels — rather than by a face that might not arrive.

## The panel that cannot be removed

`What this cannot tell you` is rendered last and is not optional. Every number here is an
**agreement** rate against a cloud answer that is itself wrong about 4% of the time past a
test gate, and none of it was ever served to anyone. A dashboard that omits that section
is a dashboard that will eventually be read as a promise.
"""

from __future__ import annotations

import argparse
import html
import json
import math
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any

from ..console import emit
from .datadir import default_log, ensure_parent
from .interval import wilson
from .savings import cost_of

AGREED = ("same_action_same_args", "same_action")




def read_jsonl(path: Path | None) -> list[dict[str, Any]]:
    if not path or not path.is_file():
        return []
    out = []
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.strip():
            try:
                out.append(json.loads(line))
            except ValueError:
                continue          # a half-written last line is not a reason to show nothing
    return out


def kerna_budgets(db_path: Path | None) -> dict[str, Any]:
    """How close the runtime's bounded runs came to their limits.

    Every Kerna run is capped -- wall clock, tool calls, LLM calls, dollars, output
    bytes -- and it records where it got to in `events.budget_snapshot_json`. None of
    that reached the report, so a page that answers "what did this cost" could not
    answer "and what stops it costing more", which is the first question anyone
    responsible for the bill actually asks.

    Read-only and best-effort, like the governance panel beside it: a missing database
    means this section says nothing rather than the page failing to render.
    """
    empty: dict[str, Any] = {"tasks": 0, "peak": {}, "totals": {}}
    if db_path is None or not Path(db_path).exists():
        return empty

    import sqlite3

    try:
        with sqlite3.connect(f"file:{Path(db_path).as_posix()}?mode=ro", uri=True) as conn:
            snapshots = [
                row[0] for row in conn.execute(
                    "SELECT budget_snapshot_json FROM events "
                    "WHERE budget_snapshot_json IS NOT NULL ORDER BY sequence"
                ).fetchall()
            ]
            tasks = conn.execute(
                "SELECT COUNT(*), COALESCE(SUM(cost_estimate), 0), "
                "COALESCE(SUM(tokens_used), 0), COALESCE(MAX(duration_secs), 0) "
                "FROM tasks"
            ).fetchone()
    except Exception:  # noqa: BLE001
        return empty

    # The *peak* each counter reached, not the last value. A run that spent its budget
    # and then finished cheaply would otherwise look untroubled, and the whole point of
    # showing a bound is knowing how near anything came to it.
    peak: dict[str, float] = {}
    for blob in snapshots:
        try:
            row = json.loads(blob)
        except ValueError:
            continue
        for key, value in (row or {}).items():
            if isinstance(value, (int, float)):
                peak[key] = max(peak.get(key, 0), value)

    count, cost, tokens, longest = tasks or (0, 0, 0, 0)
    return {
        "tasks": int(count),
        "peak": peak,
        "totals": {"cost_usd": float(cost), "tokens": int(tokens),
                   "longest_seconds": float(longest)},
    }


def kerna_events(db_path: Path | None) -> list[dict]:
    """Policy decisions from Kerna's own audit trail, keyed by the shared turn id.

    The third log. `recorder.py` records what a turn cost, `explore.py` records whether a
    local model would have agreed, and Kerna's SQLite trail records what its policy
    decided -- and until the correlation header existed, none of the three could be
    joined to the others.

    Read-only and best-effort: a missing or unreadable database means the governance
    panel reports nothing rather than the page failing. A report that refuses to render
    because one optional input is absent is a report nobody runs twice.

    `policy_decision` is what the POLICY decided, which in audit mode is not what was
    enforced -- so `enforced` is carried alongside it and the panel says which.
    """
    if db_path is None or not Path(db_path).exists():
        return []

    import sqlite3

    try:
        with sqlite3.connect(f"file:{Path(db_path).as_posix()}?mode=ro", uri=True) as conn:
            rows = conn.execute(
                "SELECT task_id, tool, policy_decision, payload_json FROM events "
                "WHERE event_type = 'tool.policy.checked' ORDER BY sequence"
            ).fetchall()
    except Exception:  # noqa: BLE001
        return []

    out: list[dict] = []
    for task_id, tool, decision, payload in rows:
        enforced = True
        if payload:
            try:
                enforced = json.loads(payload).get("enforced", True) is not False
            except ValueError:
                pass
        out.append({
            "turn": task_id,
            "tool": tool,
            "decision": decision,
            "enforced": enforced,
        })
    return out


# The fields that make two shadow rows comparable. Every one of these already travels
# on the row, so a cohort key can be derived from data that exists rather than waiting
# for new instrumentation -- which means tonight's logs group correctly too.
COHORT_FIELDS = (
    "local_model", "local_decoding", "local_tool_transport",
    "tool_policy", "local_tool_choice", "local_tool_prompt", "class_version",
)


def cohort_key(row: dict) -> str:
    """What experiment this row belongs to.

    Pooling rows from different configurations produces one agreement rate that
    describes no configuration. A 16k context run and a 24k one, or thinking on and
    off, are different experiments wearing the same file extension -- and a single
    number over both is the corpus error this project has already made twice.
    """
    return " · ".join(
        f"{f.replace('local_', '')}={row.get(f)}"
        for f in COHORT_FIELDS if row.get(f) is not None
    ) or "unconfigured"


def split_experiments(explore: list[dict]) -> dict[str, list[dict]]:
    """Group shadow rows by configuration, newest-first by insertion."""
    out: dict[str, list[dict]] = {}
    for row in explore:
        if row.get("record") != "explore":
            continue
        out.setdefault(cohort_key(row), []).append(row)
    return out


def gather(
    traffic: list[dict],
    explore: list[dict],
    ledger: dict,
    governance: list[dict] | None = None,
    budgets: dict[str, Any] | None = None,
) -> dict[str, Any]:
    """Everything the page needs, joined on the turn id."""
    requests = [r for r in traffic if r.get("record") == "request"]
    attempts = [r for r in explore if r.get("record") == "explore"]
    gates = [r for r in explore if r.get("record") == "gate"]

    cost_by_turn: dict[str, float] = {}
    spend = tokens = fresh = written = cached = produced = 0.0
    unpriced = 0
    by_model: dict[str, float] = defaultdict(float)
    for row in requests:
        usage = (row.get("response") or {}).get("usage") or {}
        fresh += usage.get("input_tokens") or 0
        written += usage.get("cache_creation_input_tokens") or 0
        cached += usage.get("cache_read_input_tokens") or 0
        produced += usage.get("output_tokens") or 0
        cost = cost_of(usage, row.get("model"))
        if cost is None:
            unpriced += 1
            continue
        spend += cost
        by_model[str(row.get("model"))] += cost
        if row.get("turn"):
            cost_by_turn[row["turn"]] = cost_by_turn.get(row["turn"], 0.0) + cost
    tokens = fresh + written + cached + produced

    # `outcome` is what the *shadow attempt* concluded; `check_infrastructure` is what
    # the validation step concluded. Only the second was consulted, so a shadow that
    # never reached the model -- a read timeout, a dead server -- was not counted as
    # infrastructure at all. On a real run that hid 8 of 18 attempts, 44%, and left the
    # dominant outcome of the session unreported.
    infrastructure = [a for a in attempts
                      if a.get("outcome") == "infrastructure_failure"
                      or a.get("check_infrastructure")]
    infra_turns = {id(a) for a in infrastructure}

    scored = [a for a in attempts
              if a.get("agreement") not in (None, "unscorable")
              and id(a) not in infra_turns]
    agreed = [a for a in scored if a.get("agreement") in AGREED]
    # Only spend on turns we actually compared may be called recoverable.
    recoverable = sum(cost_by_turn.get(a.get("turn") or "", 0.0) for a in agreed)

    by_class: dict[str, list[int]] = defaultdict(lambda: [0, 0])
    for a in scored:
        cls = str(a.get("task_class") or "unknown")
        by_class[cls][0] += 1
        by_class[cls][1] += a.get("agreement") in AGREED

    # The third log, joined on the same key. `policy_decision` is what the POLICY said;
    # in audit mode that is deliberately not what was enforced, so the two are counted
    # separately -- reporting them together would let rung 1 read as protection.
    governance = governance or []
    gov_denied = [g for g in governance
                  if "den" in str(g.get("decision") or "").lower()]
    gov_enforced = [g for g in gov_denied if g.get("enforced")]
    gov_observed = [g for g in gov_denied if not g.get("enforced")]
    gov_turns = {g.get("turn") for g in governance if g.get("turn")}
    # Turns where all three logs say something about the same piece of work. This is the
    # number that says the product is one system rather than three files.
    turns_cost_and_policy = gov_turns & set(cost_by_turn)
    turns_in_all_three = turns_cost_and_policy & {
        a.get("turn") for a in attempts if a.get("turn")
    }

    denials = [g for g in gates if g.get("decision") == "deny"]
    return {
        "requests": len(requests),
        "agentic": sum(1 for r in requests
                       if r.get("n_tools", 0) and r.get("has_tool_results")),
        "spend": spend, "unpriced": unpriced, "by_model": dict(by_model),
        "tokens": {"fresh": fresh, "written": written, "cached": cached, "out": produced,
                   "total": tokens},
        "attempts": len(attempts), "scored": len(scored), "agreed": len(agreed),
        "recoverable": recoverable,
        "context_ineligible": sum(1 for a in attempts
                                  if a.get("outcome") == "context_ineligible"),
        "translation_ineligible": sum(1 for a in attempts
                                      if a.get("outcome") == "translation_ineligible"),
        "infrastructure": len(infrastructure),
        # Every outcome the explorer can record, counted separately. Collapsing these
        # into "agreement" and "not comparable" threw away the distinction between a
        # model that answered in prose, a turn too large to send, and a server that
        # never replied -- three different problems with three different owners.
        "outcomes": dict(Counter(
            a.get("outcome") or "unrecorded" for a in attempts)),
        # What each side actually produced. The explorer records all of this and the
        # page showed none of it -- so a session where the cloud proposed two tool calls
        # and the local model proposed zero structured actions read as "no scorable
        # comparisons yet", which describes the symptom and hides the cause.
        "actions": {
            "cloud_offered": sum(1 for a in attempts if a.get("cloud_tool")),
            "local_structured": sum(
                1 for a in attempts if a.get("local_structured_tool_call")),
            "local_prose": sum(
                1 for a in attempts
                if a.get("local_text_present") and not a.get("local_structured_tool_call")
                and a.get("outcome") == "converged"),
            "local_stopped_early": sum(
                1 for a in attempts if a.get("local_stop_reason") == "length"),
            "local_abstained": sum(1 for a in attempts if a.get("local_abstained")),
            "formats": dict(Counter(
                a.get("local_action_format") for a in attempts
                if a.get("local_action_format"))),
        },
        # Latency of shadows that *completed*. A timeout is not a slow answer; it is a
        # different outcome, and averaging it in hides both -- a first version of this
        # panel reported a 120.2s median from eight read timeouts and three real
        # completions, which describes neither population.
        "latency_ms": sorted(
            a["elapsed_ms"] for a in attempts
            if isinstance(a.get("elapsed_ms"), (int, float))
            and a.get("outcome") != "infrastructure_failure"),
        "timeouts": sum(
            1 for a in attempts
            if a.get("outcome") == "infrastructure_failure"
            and "timeout" in str(a.get("error", "")).lower()),
        # Conversation depth, because lossy turns cluster late and the clean cohort
        # skews early -- a sampling bias in the local model's favour (047).
        "depths": sorted(
            a["n_messages"] for a in attempts
            if isinstance(a.get("n_messages"), int)),
        # Per configuration, never pooled. A rate over two different context sizes or
        # two decoding modes describes neither of them.
        "experiments": {
            key: {
                "attempts": len(rows),
                "outcomes": dict(Counter(
                    r.get("outcome") or "unrecorded" for r in rows)),
            }
            for key, rows in split_experiments(attempts).items()
        },
        "by_class": dict(by_class),
        "budgets": budgets or {"tasks": 0, "peak": {}, "totals": {}},
        "gate_seen": len(gates), "denials": len(denials),
        "denied_tools": Counter(g.get("tool") for g in denials),
        "would_deny": sum(1 for g in gates if g.get("enforced") is False),
        "cohorts": ledger.get("cohorts") or {},
        "joined": sum(1 for a in attempts if a.get("turn") in cost_by_turn),
        "gov_checks": len(governance),
        "gov_denied_enforced": len(gov_enforced),
        "gov_denied_observed": len(gov_observed),
        "gov_turns": len(gov_turns),
        "turns_cost_and_policy": len(turns_cost_and_policy),
        "turns_in_all_three": len(turns_in_all_three),
    }


# ------------------------------------------------------------------ rendering

def _e(text: Any) -> str:
    return html.escape(str(text))


def _tile(label: str, value: str, note: str = "", tone: str = "") -> str:
    cls = f"tile{' tile--' + tone if tone else ''}"
    # The note is built outside the f-string. A backslash inside an f-string *expression*
    # is a SyntaxError before Python 3.12, and because this module is imported lazily the
    # error surfaced only at the end of a completed run -- on the line that was about to
    # print its results.
    note_html = f'<p class="tile__note">{note}</p>' if note else ""
    return (f'<div class="{cls}"><p class="tile__label">{_e(label)}</p>'
            f'<p class="tile__value">{value}</p>'
            f'{note_html}</div>')


def _empty(message: str) -> str:
    return f'<p class="empty">{_e(message)}</p>'


def render(d: dict[str, Any], *, title: str) -> str:
    spend = d["spend"]
    tok = d["tokens"]

    # --- summary strip: the answer before the detail
    # A tile shows a dash when nothing was measured, never a zero. "$0.00" reads as
    # "we looked and you spent nothing"; the truth is that nobody has looked yet, and
    # those are opposite claims to put in front of a budget holder.
    tiles = [_tile("Spend observed", f"${spend:,.2f}" if d["requests"] else "—",
                   f"{d['requests']:,} requests · {d['agentic']:,} agentic"
                   if d["requests"] else "no traffic recorded")]
    if d["scored"]:
        rate = d["agreed"] / d["scored"]
        lo, hi = wilson(d["agreed"], d["scored"])
        tiles.append(_tile("Local agreement", f"{rate:.0%}",
                           f"{d['agreed']}/{d['scored']} scored · {lo:.0%}–{hi:.0%}",
                           "good" if lo > 0.25 else "warn"))
        tiles.append(_tile("Recoverable", f"${d['recoverable']:,.2f}",
                           "spend on turns the laptop matched"))
    else:
        tiles.append(_tile("Local agreement", "—", "nothing compared yet"))
        tiles.append(_tile("Recoverable", "—", "needs a comparison first"))
    tiles.append(_tile("Blocked by policy",
                       f"{d['denials']:,}" if d["gate_seen"] else "—",
                       f"{d['gate_seen']:,} tool calls seen" if d["gate_seen"]
                       else "the gate has not run",
                       "stop" if d["denials"] else ""))

    # --- where the money goes
    if tok["total"]:
        rows = []
        for label, n, mult in (("fresh input", tok["fresh"], 1.0),
                               ("cache write", tok["written"], 1.25),
                               ("cache read", tok["cached"], 0.10),
                               ("output", tok["out"], None)):
            share = n / tok["total"]
            rows.append(
                f'<tr><td>{label}</td><td class="num">{n:,.0f}</td>'
                f'<td class="num">{share:.1%}</td>'
                f'<td><span class="bar" style="--w:{share:.4f}"></span></td></tr>')
        carry = (tok["written"] + tok["cached"]) / tok["total"]
        money = ("<p class=\"note\">Most of this bill is <strong>carrying context</strong>, "
                 "not generating answers. A smaller model makes the cheap part cheaper.</p>"
                 if carry > 0.6 else "")
        tokens_panel = (
            '<div class="scroll"><table><thead><tr><th>tokens</th><th class="num">count</th>'
            '<th class="num">share</th><th></th></tr></thead><tbody>'
            + "".join(rows) + "</tbody></table></div>" + money)
    else:
        tokens_panel = _empty("No usage recorded yet.")

    # --- what the local model would have done
    if d["by_class"]:
        rows = []
        for cls, (n, ok) in sorted(d["by_class"].items(), key=lambda kv: -kv[1][0]):
            lo, _ = wilson(ok, n)
            tone = "good" if lo > 0.25 else ("warn" if ok else "stop")
            rows.append(f'<tr><td><code>{_e(cls)}</code></td>'
                        f'<td class="num">{n}</td><td class="num">{ok}</td>'
                        f'<td class="num">{ok / n:.0%}</td>'
                        f'<td><span class="pill pill--{tone}">lb {lo:.0%}</span></td></tr>')
        agreement_panel = ('<div class="scroll"><table><thead><tr><th>task class</th>'
                           '<th class="num">scored</th><th class="num">agreed</th>'
                           '<th class="num">rate</th><th>lower bound</th></tr></thead>'
                           '<tbody>' + "".join(rows) + '</tbody></table></div>')
    else:
        agreement_panel = _empty(
            "Nothing has been compared. Run the sidecar with --explore pointed at a local "
            "model; it serves nothing and cannot affect a request.")

    # --- attempts that produced no verdict
    blocked = []
    if d["context_ineligible"]:
        blocked.append(f"<li><strong>{d['context_ineligible']}</strong> turns exceeded the "
                       f"local context window — not a model failure, a configuration that "
                       f"was never eligible to attempt them</li>")
    if d["translation_ineligible"]:
        blocked.append(f"<li><strong>{d['translation_ineligible']}</strong> turns were "
                       f"attempted but cannot be compared — the dialect translation could "
                       f"not carry part of what the cloud saw, so a disagreement would be "
                       f"as likely ours as the model's</li>")
    if d["infrastructure"]:
        blocked.append(f"<li><strong>{d['infrastructure']}</strong> checks failed for "
                       f"infrastructure reasons — ours, not the model's, and excluded from "
                       f"every rate above</li>")
    unscored = (d["attempts"] - d["scored"] - d["context_ineligible"]
                - d["translation_ineligible"] - d["infrastructure"])
    if unscored > 0:
        blocked.append(f"<li><strong>{unscored}</strong> attempts were not comparable — "
                       f"one side answered in prose, or the turn was mid-sequence</li>")
    if blocked:
        blocked_panel = "<ul class=\"list\">" + "".join(blocked) + "</ul>"
    elif d["attempts"]:
        blocked_panel = _empty("Every attempt produced a verdict.")
    else:
        # Vacuously true of zero attempts, and it reads as a pass. Same family as
        # showing $0.00 spend for traffic nobody recorded: an absence rendered as a
        # result, which is the one thing this page must never do.
        blocked_panel = _empty("No attempts have been made.")

    # --- governance
    # What each side actually produced. "No scorable comparisons yet" describes the
    # symptom; this describes the cause, and the two are not the same sentence.
    acts = d.get("actions") or {}
    lat = d.get("latency_ms") or []
    if acts.get("cloud_offered") or acts.get("local_structured") or lat:
        def _pct(xs, q):
            return xs[min(len(xs) - 1, int(len(xs) * q))] / 1000 if xs else 0.0

        fmt = ", ".join(f"{html.escape(str(k))} &times;{v}"
                        for k, v in sorted((acts.get("formats") or {}).items()))
        action_panel = (
            f'<table><thead><tr><th></th><th>cloud</th><th>local</th></tr></thead>'
            f'<tbody>'
            f'<tr><td>proposed a tool call</td>'
            f'<td class="num">{acts.get("cloud_offered", 0):,}</td>'
            f'<td class="num">{acts.get("local_structured", 0):,}</td></tr>'
            f'<tr><td>answered in prose instead</td><td class="num">&mdash;</td>'
            f'<td class="num">{acts.get("local_prose", 0):,}</td></tr>'
            f'<tr><td>stopped at the token cap first</td><td class="num">&mdash;</td>'
            f'<td class="num">{acts.get("local_stopped_early", 0):,}</td></tr>'
            f'<tr><td>declined to act</td><td class="num">&mdash;</td>'
            f'<td class="num">{acts.get("local_abstained", 0):,}</td></tr>'
            f'</tbody></table>'
            + (f'<p class="note">Local action formats seen: {fmt}.</p>' if fmt else "")
            + (f'<p class="note">Latency of the {len(lat):,} shadow'
               f'{"s" if len(lat) != 1 else ""} that <em>completed</em>: median '
               f'<strong>{_pct(lat, 0.5):.1f}s</strong>, p95 '
               f'<strong>{_pct(lat, 0.95):.1f}s</strong>, slowest '
               f'{lat[-1] / 1000:.1f}s.</p>' if lat else "")
            + (f'<p class="note">{d.get("timeouts", 0):,} further attempt'
               f'{"s" if d.get("timeouts", 0) != 1 else ""} timed out and are counted '
               f'separately &mdash; a timeout is a different outcome from a slow '
               f'answer, and averaging the two describes neither.</p>'
               if d.get("timeouts") else "")
            + (f'<p class="note"><strong>The cloud proposed '
               f'{acts["cloud_offered"]:,} tool call'
               f'{"s" if acts["cloud_offered"] != 1 else ""} and the local model '
               f'produced no structured action at all.</strong> Agreement cannot be '
               f'computed from that, and the reason is the model’s output format '
               f'rather than its choice of tool.</p>'
               if acts.get("cloud_offered") and not acts.get("local_structured") else "")
        )
    else:
        action_panel = _empty("No shadow attempts recorded.")

    # Every outcome, named, with whose problem it is. The page used to report agreement
    # and "not comparable", which put a model answering in prose, a turn too large to
    # send, and a server that never replied into one bucket -- three different problems
    # with three different owners.
    OWNER = {
        "converged": ("the model produced an action", "model"),
        "did_not_converge": ("the model produced no action", "model"),
        "infrastructure_failure": ("the request never completed", "ours"),
        "context_ineligible": ("the turn was too large to send", "ours"),
        "translation_ineligible": ("the dialect could not carry the turn", "ours"),
        "action_space_ineligible": ("the local model was never offered that tool",
                                    "ours"),
    }
    outcomes = d.get("outcomes") or {}
    if outcomes:
        total = sum(outcomes.values())
        rows = "".join(
            f"<tr><td>{html.escape(OWNER.get(k, (k, '-'))[0])}</td>"
            f"<td class=\"num\">{v:,}</td>"
            f"<td class=\"num\">{v / total:.0%}</td>"
            f"<td>{OWNER.get(k, (k, 'unknown'))[1]}</td></tr>"
            for k, v in sorted(outcomes.items(), key=lambda kv: -kv[1])
        )
        ours = sum(v for k, v in outcomes.items()
                   if OWNER.get(k, ("", ""))[1] == "ours")
        outcome_panel = (
            f'<table><thead><tr><th>what happened</th><th>n</th><th>share</th>'
            f'<th>whose problem</th></tr></thead><tbody>{rows}</tbody></table>'
            + (f'<p class="note"><strong>{ours:,} of {total:,}</strong> shadows ended '
               f'for reasons that are ours rather than the model&rsquo;s. A comparison '
               f'rate computed over the rest is a statement about the turns that '
               f'survived our own plumbing.</p>' if ours else "")
        )
    else:
        outcome_panel = _empty("No shadow attempts recorded.")

    budgets = d.get("budgets") or {}
    peak = budgets.get("peak") or {}
    if budgets.get("tasks"):
        totals = budgets["totals"]
        # Peak, not final: a run that spent its budget and then finished cheaply would
        # otherwise look untroubled, and the point of showing a bound is knowing how
        # close anything came to it.
        rows = "".join(
            f"<tr><td>{html.escape(k.replace('_used', '').replace('_', ' '))}</td>"
            f"<td class=\"num\">{v:,.4g}</td></tr>"
            for k, v in sorted(peak.items())
        )
        budget_panel = (
            f'<p class="note"><strong>{budgets["tasks"]:,}</strong> bounded run'
            f'{"s" if budgets["tasks"] != 1 else ""} recorded. '
            f'Every one is capped on wall clock, tool calls, model calls, dollars and '
            f'output size; a run that reaches a cap stops and says which.</p>'
            f'<table><thead><tr><th>counter</th><th>highest reached</th></tr></thead>'
            f'<tbody>{rows}</tbody></table>'
            f'<p class="note">Total spend across those runs '
            f'<strong>${totals["cost_usd"]:,.4f}</strong>, '
            f'{totals["tokens"]:,} tokens, longest run '
            f'{totals["longest_seconds"]:,.0f}s.</p>'
        )
    else:
        budget_panel = _empty(
            "No bounded runs recorded. Pass --kerna-db to include the runtime's own "
            "budget accounting.")

    if d["gate_seen"]:
        items = "".join(
            f'<tr><td><code>{_e(tool)}</code></td><td class="num">{n}</td></tr>'
            for tool, n in d["denied_tools"].most_common(8))
        gate_panel = (f'<p class="note"><strong>{d["denials"]:,}</strong> of '
                      f'{d["gate_seen"]:,} tool calls were blocked.'
                      + (f' A further <strong>{d["would_deny"]}</strong> would have been '
                         f'blocked if enforcement were on.' if d["would_deny"] else "")
                      + '</p>'
                      + (f'<div class="scroll"><table><thead><tr><th>tool</th>'
                         f'<th class="num">blocked</th></tr></thead><tbody>{items}'
                         f'</tbody></table></div>' if items else ""))
    else:
        gate_panel = _empty("No tool calls have passed the policy gate yet.")

    # --- the runtime's own policy decisions (Kerna's audit trail, third log)
    if d["gov_checks"]:
        enforced, observed = d["gov_denied_enforced"], d["gov_denied_observed"]
        checks = d["gov_checks"]
        parts = [f'<p class="note"><strong>{checks:,}</strong> policy '
                 f'{"decision was" if checks == 1 else "decisions were"} recorded by '
                 f'the runtime.']
        if enforced:
            parts.append(f' <strong>{enforced}</strong> denied and enforced.')
        if observed:
            # Kept separate on purpose. Rung 1 records a denial and allows the action
            # through; adding the two together would let audit mode read as protection,
            # which is the exact overclaim the mode exists to avoid.
            parts.append(
                f' <strong>{observed}</strong> would have been denied but '
                f'{"ran" if observed == 1 else "ran"} anyway — the runtime was in '
                f'audit mode, which enforces nothing.')
        if not enforced and not observed:
            parts.append(" None were denied.")
        parts.append("</p>")
        gov_panel = "".join(parts)
    else:
        gov_panel = _empty(
            "The runtime's audit trail was not supplied. Pass --kerna-db to join what "
            "policy decided to what the turn cost.")

    # --- the ledger
    if d["cohorts"]:
        rows = []
        for key, c in sorted(d["cohorts"].items()):
            trials, agree = c.get("trials", 0), c.get("agreements", 0)
            lo, _ = wilson(agree, trials)
            if c.get("promoted"):
                state, tone = "promoted", "good"
            elif c.get("demoted_reason"):
                state, tone = "demoted", "stop"
            elif trials < 30:
                state, tone = f"learning · {30 - trials} more", "warn"
            else:
                state, tone = "below bar", ""
            rows.append(f'<tr><td><code>{_e(key.replace("|", " · "))}</code></td>'
                        f'<td class="num">{trials}</td><td class="num">{lo:.0%}</td>'
                        f'<td><span class="pill pill--{tone}">{_e(state)}</span></td></tr>')
        ledger_panel = ('<div class="scroll"><table><thead><tr><th>cohort</th>'
                        '<th class="num">trials</th><th class="num">lower bound</th>'
                        '<th>state</th></tr></thead><tbody>' + "".join(rows)
                        + '</tbody></table></div>'
                        '<p class="note">A promotion authorises <strong>attempting</strong> '
                        'locally, not trusting the answer. Every local answer is still '
                        'checked and still escalates.</p>')
    else:
        ledger_panel = _empty("No class has earned the right to be attempted locally. "
                              "Every request goes to the cloud.")

    join_note = ""
    if d["attempts"] and not d["joined"]:
        join_note = ('<p class="note note--warn">No exploration row could be joined to a '
                     'recorded request, so per-turn cost cannot be attributed. Check that '
                     'the turn id is reaching both logs.</p>')

    panels = [
        ("Where the money goes", tokens_panel, ""),
        ("What the laptop would have done", agreement_panel, join_note),
        ("Attempts with no verdict", blocked_panel, ""),
        ("What each side produced", action_panel, ""),
        ("Why each shadow ended", outcome_panel, ""),
        ("What bounds the runtime", budget_panel, ""),
        ("What policy stopped", gate_panel, ""),
        ("What the runtime decided", gov_panel, ""),
        ("What has earned autonomy", ledger_panel, ""),
    ]
    panel_html = "".join(
        f'<section class="panel"><h2>{_e(name)}</h2>{body}{extra}</section>'
        for name, body, extra in panels)

    unpriced_note = (f'<p class="note note--warn">{d["unpriced"]} requests are excluded — '
                     f'no published rate for their model. The total above is a floor.</p>'
                     if d["unpriced"] else "")

    return _PAGE.format(title=_e(title), tiles="".join(tiles), panels=panel_html,
                        unpriced=unpriced_note)


_PAGE = """<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<style>
  /* Instrument-panel neutrals: a cool grey biased toward the accent, not a default grey.
     Semantic colours are separate from the accent so "needs attention" never reads as
     "this is the brand colour". */
  :root {{
    --ground:#F4F6F8; --panel:#FFFFFF; --ink:#131820; --ink-soft:#5A6472;
    --ink-faint:#8792A0; --rule:#DDE3EA; --rule-soft:#EBEFF3;
    --accent:#1F5F8B; --accent-wash:#E7F0F6;
    --good:#2D7A5F; --good-wash:#E4F1EB;
    --warn:#8A6412; --warn-wash:#F6EEDC;
    --stop:#A33C34; --stop-wash:#F6E6E4;
    --sans:"Segoe UI Variable Display","Segoe UI",system-ui,-apple-system,"Helvetica Neue",sans-serif;
    --mono:"Cascadia Mono","SF Mono",Consolas,ui-monospace,monospace;
  }}
  @media (prefers-color-scheme:dark) {{
    :root:not([data-theme="light"]) {{
      --ground:#0E1215; --panel:#171D23; --ink:#E4E9EE; --ink-soft:#A2AEBB;
      --ink-faint:#76828F; --rule:#2A333C; --rule-soft:#212930;
      --accent:#5BA3D0; --accent-wash:#152833;
      --good:#5FBF97; --good-wash:#15291F;
      --warn:#D3A44A; --warn-wash:#2B2415;
      --stop:#DE7A70; --stop-wash:#2E1B19;
    }}
  }}
  :root[data-theme="dark"] {{
    --ground:#0E1215; --panel:#171D23; --ink:#E4E9EE; --ink-soft:#A2AEBB;
    --ink-faint:#76828F; --rule:#2A333C; --rule-soft:#212930;
    --accent:#5BA3D0; --accent-wash:#152833;
    --good:#5FBF97; --good-wash:#15291F;
    --warn:#D3A44A; --warn-wash:#2B2415;
    --stop:#DE7A70; --stop-wash:#2E1B19;
  }}
  *{{box-sizing:border-box}}
  body{{background:var(--ground);color:var(--ink);font-family:var(--sans);
       font-size:15px;line-height:1.55;margin:0;padding:0 1.5rem 5rem;
       -webkit-font-smoothing:antialiased}}
  .wrap{{max-width:64rem;margin:0 auto;display:flex;flex-direction:column;gap:1.5rem}}

  header{{padding:3rem 0 0;display:flex;flex-direction:column;gap:.4rem}}
  .eyebrow{{font-family:var(--mono);font-size:.68rem;letter-spacing:.14em;
           text-transform:uppercase;color:var(--ink-faint)}}
  h1{{font-size:clamp(1.5rem,3.5vw,2rem);line-height:1.15;letter-spacing:-.02em;
     font-weight:640;margin:0;text-wrap:balance}}

  .strip{{display:grid;grid-template-columns:repeat(auto-fit,minmax(11rem,1fr));gap:1px;
         background:var(--rule);border:1px solid var(--rule);border-radius:3px;
         overflow:hidden}}
  .tile{{background:var(--panel);padding:1.1rem 1.2rem;display:flex;
        flex-direction:column;gap:.15rem;border-top:2px solid transparent}}
  .tile--good{{border-top-color:var(--good)}}
  .tile--warn{{border-top-color:var(--warn)}}
  .tile--stop{{border-top-color:var(--stop)}}
  .tile__label{{font-family:var(--mono);font-size:.66rem;letter-spacing:.12em;
               text-transform:uppercase;color:var(--ink-faint);margin:0}}
  .tile__value{{font-family:var(--mono);font-variant-numeric:tabular-nums;
               font-size:1.65rem;font-weight:600;letter-spacing:-.02em;margin:0;
               line-height:1.1}}
  .tile__note{{font-size:.8rem;color:var(--ink-soft);margin:0}}

  .grid{{display:grid;grid-template-columns:repeat(auto-fit,minmax(21rem,1fr));gap:1.5rem}}
  .panel{{background:var(--panel);border:1px solid var(--rule);border-radius:3px;
         padding:1.3rem 1.4rem;display:flex;flex-direction:column;gap:.9rem}}
  .panel h2{{font-family:var(--mono);font-size:.7rem;letter-spacing:.12em;
            text-transform:uppercase;color:var(--accent);font-weight:600;margin:0}}

  .scroll{{overflow-x:auto}}
  table{{width:100%;border-collapse:collapse;font-size:.88rem}}
  th{{text-align:left;font-family:var(--mono);font-size:.64rem;letter-spacing:.1em;
     text-transform:uppercase;color:var(--ink-faint);font-weight:600;
     padding:0 .7rem .5rem 0;border-bottom:1px solid var(--rule)}}
  td{{padding:.5rem .7rem .5rem 0;border-bottom:1px solid var(--rule-soft);
     color:var(--ink-soft);vertical-align:middle}}
  td:first-child{{color:var(--ink)}}
  .num{{font-family:var(--mono);font-variant-numeric:tabular-nums;text-align:right;
       white-space:nowrap;color:var(--ink)}}
  code{{font-family:var(--mono);font-size:.85em}}

  .bar{{display:block;height:6px;min-width:1px;width:calc(var(--w)*100%);
       background:var(--accent);border-radius:1px}}
  .pill{{display:inline-block;font-family:var(--mono);font-size:.66rem;
        letter-spacing:.06em;padding:.16rem .45rem;border-radius:2px;
        background:var(--rule-soft);color:var(--ink-soft);white-space:nowrap}}
  .pill--good{{background:var(--good-wash);color:var(--good)}}
  .pill--warn{{background:var(--warn-wash);color:var(--warn)}}
  .pill--stop{{background:var(--stop-wash);color:var(--stop)}}

  .note{{font-size:.85rem;color:var(--ink-soft);margin:0}}
  .note--warn{{color:var(--warn)}}
  .empty{{font-size:.88rem;color:var(--ink-faint);margin:0;font-style:italic}}
  .list{{margin:0;padding-left:1.1rem;font-size:.88rem;color:var(--ink-soft);
        display:flex;flex-direction:column;gap:.35rem}}
  .list strong,.note strong{{color:var(--ink);font-variant-numeric:tabular-nums}}

  .limits{{background:transparent;border:1px dashed var(--rule);border-radius:3px;
          padding:1.3rem 1.4rem;display:flex;flex-direction:column;gap:.7rem}}
  .limits h2{{font-family:var(--mono);font-size:.7rem;letter-spacing:.12em;
             text-transform:uppercase;color:var(--ink-faint);font-weight:600;margin:0}}
  .limits ul{{margin:0;padding-left:1.1rem;font-size:.87rem;color:var(--ink-soft);
             display:flex;flex-direction:column;gap:.5rem}}
  :focus-visible{{outline:2px solid var(--accent);outline-offset:2px}}
  @media (max-width:34rem){{body{{padding:0 1rem 3rem;font-size:14px}}}}
</style>

<div class="wrap">
  <header>
    <p class="eyebrow">Agent control plane</p>
    <h1>{title}</h1>
  </header>

  <div class="strip">{tiles}</div>
  {unpriced}

  <div class="grid">{panels}</div>

  <section class="limits">
    <h2>What this cannot tell you</h2>
    <ul>
      <li><strong>Agreement is not correctness.</strong> Local answers were compared against
          the cloud's own answer, which ships a wrong answer past a test gate about 4% of the
          time. Two models can agree and both be wrong.</li>
      <li><strong>Nothing was served locally.</strong> Every answer came from your provider,
          unchanged. This measures what could have been, at no risk.</li>
      <li><strong>A saving needs a machine that can run the model.</strong> Throughput is a
          cliff, not a slope — a laptop that fits the model runs it about five times faster
          than one that does not, and integrated graphics cannot run it at all.</li>
      <li><strong>Routing a turn away does not remove all of its cost.</strong> Its context is
          still carried by the next cloud turn, arriving as a cache write. How much that
          costs is not yet measured.</li>
    </ul>
  </section>
</div>
"""


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="dashboard", description=__doc__.split("\n")[0])
    ap.add_argument("--traffic", type=Path, default=default_log("traffic.jsonl"))
    # Several, because a session produces one log per configuration and reading one at a
    # time is how a run's context gets lost. They are grouped by cohort, never pooled:
    # a rate over two context sizes or two decoding modes describes neither.
    ap.add_argument("--explore", type=Path, nargs="*",
                    default=[default_log("explore.jsonl")],
                    help="one or more shadow logs; grouped by configuration")
    ap.add_argument("--ledger", type=Path, default=default_log("ledger.json"))
    ap.add_argument("--kerna-db", type=Path, default=None,
                    help="Kerna's SQLite audit trail, joined on the shared turn id. "
                         "Without it the report has two of the three logs.")
    ap.add_argument("--title", default="Agent spend and autonomy")
    ap.add_argument("--out", type=Path, default=default_log("report.html"))
    args = ap.parse_args(argv)

    ledger: dict[str, Any] = {}
    if args.ledger.is_file():
        try:
            ledger = json.loads(args.ledger.read_text(encoding="utf-8"))
        except ValueError:
            ledger = {}

    data = gather(
        read_jsonl(args.traffic),
        [row for path in (args.explore or []) for row in read_jsonl(path)],
        ledger,
        governance=kerna_events(args.kerna_db),
        budgets=kerna_budgets(args.kerna_db),
    )
    ensure_parent(args.out).write_text(render(data, title=args.title), encoding="utf-8")

    emit(f"dashboard written: {args.out}")
    emit(f"  {data['requests']} requests · ${data['spend']:,.2f} · "
         f"{data['scored']} scored attempts · {data['denials']} blocked")
    if not data["requests"]:
        emit("  no traffic recorded — the page will say so rather than show zeros")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
