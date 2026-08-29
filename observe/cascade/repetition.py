"""Did the local model advance the task, or restate something already done?

Observed in the second analysed cohort, and it is the sort of thing a rate would never
have shown:

    turn 4   Claude   grep -rn "def default_log" -A 20 evals/m0/cascade/*.py   "Inspect default_log"
    turn 6   Qwen     grep -rn "def default_log" -A 10 evals/m0/cascade/*.py   "Inspect default_log"

Same description, near-identical command. At turn 6 the local model proposed the action
Claude had already taken at turn 4 — an action sitting in its own context, with the
result attached.

If that is systematic it is a far more useful diagnosis than a disagreement rate. A model
that re-runs a completed step is not making a *different* judgement; it is failing to
notice the step happened, which points at prompt shape rather than at capability. It also
predicts exactly the pattern the cohort shows: intents that look plausible in isolation
and never advance the task.

## The control is the whole point

Claude repeats itself too — re-greping a file after an edit is ordinary, correct
behaviour. So both sides are measured against the same context, and only the *gap*
between them is evidence. Measuring the local model alone would have produced a number
with nothing to compare it to, which is how a normal behaviour becomes a pathology in a
slide.

## Two strengths, because exact repetition is rare

`exact` is the same tool with the same arguments. `equivalent` uses the frozen
`semantic-v1` reading, so `-A 20` versus `-A 10` still counts — which is what the
observed case actually looks like. Both are recorded; neither is a verdict.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any

from .semantic import compare_semantic


@dataclass(frozen=True)
class PriorAction:
    tool: str
    args: dict[str, Any]

    def key(self) -> str:
        return json.dumps({"t": self.tool, "a": self.args}, sort_keys=True,
                          separators=(",", ":"))


def prior_actions(payload: dict[str, Any] | None) -> list[PriorAction]:
    """Every tool call already present in the conversation, in either dialect.

    These are actions the model can see, most with a result attached. Proposing one again
    is not a new decision.
    """
    out: list[PriorAction] = []
    if not isinstance(payload, dict):
        return out

    for message in payload.get("messages") or []:
        if not isinstance(message, dict):
            continue

        content = message.get("content")
        if isinstance(content, list):                        # Anthropic
            for block in content:
                if (isinstance(block, dict)
                        and block.get("type") == "tool_use"
                        and isinstance(block.get("name"), str)):
                    args = block.get("input")
                    out.append(PriorAction(block["name"],
                                           args if isinstance(args, dict) else {}))

        for call in message.get("tool_calls") or []:         # OpenAI
            if not isinstance(call, dict):
                continue
            fn = call.get("function") or {}
            name = fn.get("name")
            if not isinstance(name, str):
                continue
            raw = fn.get("arguments")
            if isinstance(raw, str):
                try:
                    raw = json.loads(raw)
                except ValueError:
                    raw = {}
            out.append(PriorAction(name, raw if isinstance(raw, dict) else {}))

    return out


def repeats_prior(action, priors: list[PriorAction]) -> tuple[bool, bool]:
    """(exact, equivalent) — whether this action restates one already in the context.

    `equivalent` uses the frozen semantic-v1 reading, because the observed case differed
    only in a flag value and an exact test would have missed it entirely.
    """
    if action is None or not priors:
        return False, False

    exact = False
    equivalent = False
    action_key = PriorAction(action.tool, action.args).key()

    for prior in priors:
        if prior.key() == action_key:
            exact = True
            equivalent = True
            break
        if compare_semantic(prior.tool, prior.args, action.tool, action.args).equivalent:
            equivalent = True

    return exact, equivalent


def repetition_row(payload, cloud_action, local_action) -> dict[str, Any]:
    """Both sides against the same context. Only the gap between them is evidence."""
    priors = prior_actions(payload)
    local_exact, local_equiv = repeats_prior(local_action, priors)
    cloud_exact, cloud_equiv = repeats_prior(cloud_action, priors)

    return {
        "prior_actions": len(priors),
        "local_repeats_exact": local_exact,
        "local_repeats_equivalent": local_equiv,
        "cloud_repeats_exact": cloud_exact,
        "cloud_repeats_equivalent": cloud_equiv,
    }
