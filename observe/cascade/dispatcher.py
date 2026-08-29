"""The dispatcher — decides where one request runs, and defaults to "not here".

Every rule below is a way of saying no. That is deliberate: the cascade's promise is
*never worse than today*, and the cheapest way to keep it is for local execution to be
the exception a request has to qualify for, not the default it has to escape.

## The fairness gate

`LOCAL_ROUTING_ENABLED` is **False**, and the C0a fairness test is why — though no longer
for the reason first written here. The product's claim is "no worse than the cloud you
already pay for". That comparison has now been made (C0a, 20 Aug 2026), and it came back
**inconclusive in a specific and useful way**:

| arm | slips a wrong answer past the gate | 95% lb |
|---|---|---|
| local | 8.1% | 86.1% precision |
| cloud | 3.8% | 92.0% precision |

The gap is +4.3 points to the cloud, **95% CI on the difference [−1.2, +9.8]**. The kill
criterion (cloud cleaner by ≥ 5 at the lower bound) did not fire. But the interval
contains zero *and* contains 5, so the run cannot show parity either — HumanEval's 164
problems put 11 failures against 6, and no amount of care extracts a tighter answer from
that. It is a limit of the benchmark, not of the run.

So the flag stays **False as a shipped default**, and the honest reason has changed from
"we have not measured" to "we measured, and the instrument could not resolve it". A pilot
may flip it per-install, because a customer's own traffic settles at a scale this
benchmark never could — which is precisely why Decision 034 specifies ε-audits.

Comparing the two lower bounds directly (86.1% vs 92.0%) would have read as a clear cloud
win and shut the project. The interval on the *difference* is the correct test, and the
difference between those two habits is the whole result.

## Eligibility: is an agent working, or is a person reading?

Decision 031 is sound: validation needs the complete output, so a validated answer cannot
be *generated* incrementally. The original implementation read that as "never serve a
request with `stream: true`" — a proxy for "a human is waiting", taken from the request's
own declaration rather than guessed at, which felt like the disciplined choice.

**Real Claude Code traffic says it means nothing of the sort: every agentic turn streams
(6/6, Wilson LB 0.61 — C1-observe, 20 Aug 2026).** The old rule therefore refused 8 of
the 9 requests in that window, including all 6 agentic ones, and the product routed
nothing at all.

Six is a small number and the bound says so. It is enough to *reject* the old rule --
a rule that routes zero of everything it was written for is refuted by one clean turn --
and not yet enough to size the eligible slice. That number needs a pilot.

The replacement is `is_agentic`: tool definitions declared **and** a prior `tool_result`
present. Both are observed directly in the request, and together they mean the agent has
already gone away and done something — the human is waiting on a task, not reading
tokens. Decision 031 survives intact, because the local answer is still completed and
validated before its first byte is emitted; what changes is that emitting it as a stream
is now understood as a *serving* obligation rather than a reason to refuse the work.

`local_can_stream` gates that obligation. It is False until stream synthesis is built, so
the dispatcher cannot route work the serving path could not deliver.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum
from typing import Any

from .circuit import CircuitBreaker
from .classify import EDIT, Turn

# The C0a gate, as code. See the module docstring: this is not a feature toggle.
LOCAL_ROUTING_ENABLED = False


def is_agentic(payload: dict[str, Any]) -> bool:
    """Is an agent working here, or is a person reading?

    This replaces `stream: true`, which was chosen as a proxy for "a human is waiting"
    and measured to mean nothing of the sort — every real agentic turn streams (6/6,
    Wilson LB 0.61), so the old rule refused every turn of real work.

    ## Reading that measurement honestly

    The capture behind it is 40 requests, and **only 9 of them are evidence**. Before the
    `accept-encoding` fix the client could not parse our relayed stream and silently
    retried each turn blocking, so 15 of the 40 are our own bug wearing the costume of a
    client that sometimes does not stream. Counting them gives "59% of agentic turns
    stream", a number describing no client that has ever existed.

    The lesson generalises past this file: a *run* is not automatically an analysis
    window. The fix landed mid-run, so the per-run header that was added to stop exactly
    this mistake did not catch it. `data/traffic-c1-clean.jsonl` holds the 9 clean rows so
    the claim outlives the gitignored log it came from.

    Two conditions, both read from the request, both observed in real traffic:

      * **tool definitions are declared** — a chat box does not ship 77 tools
      * **a prior tool result is present**, in either dialect — the loop is already
        running, which means the human handed off some turns ago and is waiting on a
        *task*, not on tokens

    Requiring both is deliberate. Tools alone would catch the opening turn, where a
    person has just typed and may well be watching the reply appear. A `tool_result`
    means the agent has already gone away and done something, and nobody reads tool
    output as it streams.

    ## The obligation this creates

    Agentic requests stream, so serving one locally means **synthesising a stream** from a
    complete local answer. That is honest — the client asked for events and gets events,
    just produced at once rather than incrementally — and Decision 031's real constraint
    survives untouched: the answer is complete and validated *before* the first byte is
    emitted. It is not yet built, and `local_can_stream` gates on it so this rule cannot
    route work the serving path cannot deliver.
    """
    if not payload.get("tools"):
        return False
    return any(
        _carries_tool_result(m) for m in payload.get("messages") or [] if isinstance(m, dict)
    )


def _carries_tool_result(message: dict[str, Any]) -> bool:
    """Does this message hand back the result of a tool call?

    Both dialects have to be read, and they say it differently. Anthropic puts a
    `tool_result` **block** inside a user message's content list; OpenAI gives the result
    its own message with `role: "tool"`. We captured Anthropic traffic and the
    interceptor defaults to the OpenAI endpoint, so checking only the shape we happened
    to record would have reproduced the bug this predicate exists to fix -- total refusal
    -- in the other dialect, and it would have looked like a policy decision rather than
    an oversight.
    """
    if message.get("role") == "tool":                      # OpenAI
        return True
    content = message.get("content")
    return isinstance(content, list) and any(              # Anthropic
        isinstance(b, dict) and b.get("type") == "tool_result" for b in content
    )


class Route(Enum):
    LOCAL = "local"      # attempt locally now; escalate on stall
    CLOUD = "cloud"      # straight upstream, exactly as if we did not exist
    EXPLORE = "explore"  # attempt locally on the idle budget, for evidence only


@dataclass(frozen=True)
class Verdict:
    route: Route
    reason: str          # always populated — an unexplained routing decision is a bug

    @property
    def runs_locally_now(self) -> bool:
        return self.route is Route.LOCAL


@dataclass
class Dispatcher:
    """Consults, in order: eligibility, the breaker, the ledger — and the fairness
    gate only where it applies, immediately before serving."""

    breaker: CircuitBreaker = field(default_factory=CircuitBreaker)
    # Injected rather than imported so the fairness gate can be flipped in a test
    # without mutating module state, and so a pilot can enable it per-install.
    local_routing_enabled: bool = LOCAL_ROUTING_ENABLED
    # Task classes with earned autonomy. Empty until the ledger exists (C2), which is
    # why nothing routes locally yet even with the gate open — evidence first, always.
    promoted_classes: frozenset[str] = frozenset()
    idle: bool = False
    # Serving a local answer to a streaming client requires synthesising SSE from a
    # complete response. Until that exists, a streaming request cannot be served locally
    # however eligible it is -- routing work the serving path cannot deliver would break
    # the client, which is the one thing this package may never do.
    # True since stream synthesis landed. A completed local answer is delivered as the
    # SSE sequence the client expects (`synth.py`), which is the delivery half Decision
    # 036 separated from generation. It was False for as long as that did not exist --
    # and while it was False, local routing could serve nothing at all, because every
    # agentic turn streams.
    #
    # This does NOT enable routing. `LOCAL_ROUTING_ENABLED` still gates that, and it is
    # still off pending a decision about C0a.
    local_can_stream: bool = True

    def decide(self, payload: dict[str, Any], *, task_class: str | None = None,
               turn: Turn | None = None) -> Verdict:
        """Route one request. Never raises: an undecidable request is a cloud request.

        `turn` comes from `classify()` and carries the sequence position as well as the
        class. Passing `task_class` alone still works and skips the checkpoint gate —
        which is the older, more permissive behaviour, so callers that want the P1.1
        finding enforced have to opt into it explicitly rather than inherit it silently.
        """
        try:
            return self._decide(payload, task_class, turn)
        except Exception as exc:  # noqa: BLE001
            # A crash in our own routing logic must cost the customer nothing.
            return Verdict(Route.CLOUD, f"dispatcher_error:{type(exc).__name__}")

    def _decide(self, payload: dict[str, Any], task_class: str | None,
                turn: Turn | None = None) -> Verdict:
        # NOTE the fairness gate is NOT first, and that is deliberate.
        #
        # It was, and it made EXPLORE unreachable: every request returned CLOUD before
        # eligibility was ever considered, so an explorer attached to this dispatcher
        # would sit empty forever and the whole of P1.2 would gather nothing. The gate
        # protects a *claim about serving* — "no worse than the cloud you already pay
        # for" — and exploration makes no such claim, because the customer's answer
        # always comes from the cloud regardless (Decision 037). Gating the safe path
        # with the rule that exists to protect the unsafe one is just a bug.
        #
        # So it moves to where it belongs: immediately before LOCAL, and nowhere else.

        # Eligibility is decided by whether an AGENT is working, not by the stream flag.
        # See `is_agentic` for why, and PHASE0-PREDICTIONS.md (C1-observe) for the
        # measurement that forced the change.
        if not is_agentic(payload):
            if payload.get("stream"):
                return Verdict(Route.CLOUD, "streaming_non_agentic:human_may_be_reading")
            return Verdict(Route.CLOUD, "non_agentic:no_tool_loop_to_join")

        if turn is not None and not task_class:
            task_class = turn.task_class

        if not task_class:
            return Verdict(Route.CLOUD, "unclassified_task")

        if not self.breaker.allows_local():
            return Verdict(Route.CLOUD, f"breaker_{self.breaker.state.value}")

        if task_class in self.promoted_classes:
            # Checkpoint gating protects SERVING. Exploration must observe both
            # checkpoint and mid-sequence turns or Decision 038 cannot be tested.
            if turn is not None and turn.task_class == EDIT and not turn.at_checkpoint:
                return Verdict(
                    Route.CLOUD,
                    f"mid_sequence:{turn.edits_since_check}_edits_since_check",
                )
            # The fairness gate, at the only point where it applies: about to serve.
            if not self.local_routing_enabled:
                return Verdict(Route.CLOUD, "local_routing_disabled:awaiting_c0a_fairness_test")
            if payload.get("stream") and not self.local_can_stream:
                return Verdict(Route.CLOUD, "eligible_but_stream_synthesis_not_built")
            return Verdict(Route.LOCAL, f"promoted:{task_class}")

        if self.idle:
            # Unpromoted classes are explored only when nobody is waiting and the
            # machine is otherwise idle: evidence is paid for with electrons, never
            # with anyone's time. Note this needs neither the fairness gate nor stream
            # synthesis, because nothing here is ever served.
            return Verdict(Route.EXPLORE, f"exploring:{task_class}")

        return Verdict(Route.CLOUD, f"unpromoted:{task_class}")


def gate_status(enabled: bool | None = None) -> str:
    """One line for operators and for the demo, so the flag is never a silent default.

    Takes the *effective* setting rather than reading the module constant, because the
    flag is now a per-install choice. Reading the constant printed "local routing
    DISABLED" directly beneath "--serve-local is on", which an operator would reasonably
    read as the flag having failed to take.
    """
    if LOCAL_ROUTING_ENABLED if enabled is None else enabled:
        return (
            "local routing ENABLED — pilot use only. C0a could not show the cloud is "
            "materially cleaner (gap +4.3 pts, CI [-1.2, +9.8]); it could not show "
            "parity either. Audit against cloud on real traffic before trusting this."
        )
    return (
        "local routing DISABLED — every request goes to the cloud, unchanged. "
        "C0a has run: local slips 8.1% of wrong answers past the gate, cloud 3.8%, "
        "gap +4.3 pts with CI [-1.2, +9.8] — no kill, but no proof of parity, so this "
        "stays off as a shipped default and may be flipped per-install for a pilot."
    )
