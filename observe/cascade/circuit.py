"""The circuit breaker — the thing that makes "fail open" true under load.

Decision 030 is absolute: local is an optimisation that may vanish at any instant, never
a dependency. A single failed local attempt is cheap and handled by escalating that one
request. **Repeated** failures are different in kind: they mean the local path is broken
right now — the model server died, the GPU is saturated, a driver update landed — and
continuing to try it converts every request into a wasted attempt plus a cloud call.

So the breaker exists to stop *trying*, not to stop *serving*. When it opens, every
request still gets answered; they just go straight to the cloud, which is the customer's
status quo. **An open breaker is invisible to the user by construction.**

## States

    CLOSED     normal. Local attempts allowed.
    OPEN       local path presumed broken. Every request goes to cloud. A cooldown runs.
    HALF_OPEN  cooldown elapsed. Exactly ONE probe attempt is allowed through; its
               outcome decides whether the breaker closes or re-opens.

The half-open probe is the part that matters and the part most implementations get
wrong. Without it, a breaker that opens during a transient blip stays open until someone
restarts something. With it, recovery is automatic and costs one request's worth of
risk — and that request escalates on failure like any other, so the cost of a wrong
guess is zero.

## Why consecutive failures rather than a rate

A rate needs a window, a window needs tuning, and tuning needs data we do not have from
real fleets yet. Consecutive failures is cruder and has one property that matters more
than precision here: it cannot be fooled by volume. Three failures in a row means the
path is broken whether the machine is doing three requests an hour or three thousand.
When fleet data exists, revisit — and record the change as a decision, not a tweak.
"""

from __future__ import annotations

import time
from dataclasses import dataclass, field
from enum import Enum

DEFAULT_FAILURE_THRESHOLD = 3
DEFAULT_COOLDOWN_S = 300.0


class BreakerState(Enum):
    CLOSED = "closed"
    OPEN = "open"
    HALF_OPEN = "half-open"


@dataclass
class CircuitBreaker:
    """Tracks local-path health. Never raises; never blocks a request from being served."""

    failure_threshold: int = DEFAULT_FAILURE_THRESHOLD
    cooldown_s: float = DEFAULT_COOLDOWN_S
    clock: object = field(default=time.monotonic)

    _consecutive_failures: int = 0
    _opened_at: float | None = None
    _probe_in_flight: bool = False
    # Counters, for the operator report. Deliberately not a decision input: a breaker
    # that changes behaviour based on its own history is much harder to reason about
    # during an incident, which is the only time anyone reads it.
    total_opens: int = 0
    total_probes: int = 0

    # ---------------------------------------------------------------- state

    @property
    def state(self) -> BreakerState:
        if self._opened_at is None:
            return BreakerState.CLOSED
        if self._elapsed_since_open() >= self.cooldown_s:
            return BreakerState.HALF_OPEN
        return BreakerState.OPEN

    def _elapsed_since_open(self) -> float:
        assert self._opened_at is not None
        return float(self.clock()) - self._opened_at  # type: ignore[operator]

    def allows_local(self) -> bool:
        """May this request attempt the local path?

        In HALF_OPEN exactly one caller gets True until that probe reports back. Without
        the in-flight guard, a burst of concurrent requests at cooldown expiry would all
        probe at once and hammer a path we already suspect is broken.
        """
        state = self.state
        if state is BreakerState.CLOSED:
            return True
        if state is BreakerState.OPEN:
            return False
        if self._probe_in_flight:
            return False
        self._probe_in_flight = True
        self.total_probes += 1
        return True

    # ---------------------------------------------------------------- outcomes

    def record_success(self) -> None:
        """A local attempt converged. Any suspicion is cleared."""
        self._consecutive_failures = 0
        self._opened_at = None
        self._probe_in_flight = False

    def record_failure(self) -> None:
        """A local attempt failed for an INFRASTRUCTURE reason.

        Only infrastructure failures belong here. A model that simply could not solve the
        task is a normal, expected outcome — that request escalates and the next one is
        still worth attempting. Feeding ordinary non-convergence into the breaker would
        open it constantly on hard task classes and silently disable the product; the
        ledger, not the breaker, is what learns which classes are worth attempting.
        """
        self._probe_in_flight = False
        self._consecutive_failures += 1
        if self._consecutive_failures >= self.failure_threshold:
            if self._opened_at is None:
                self.total_opens += 1
            self._opened_at = float(self.clock())  # type: ignore[operator]

    def record_escalation(self) -> None:
        """The model did not converge, but nothing was broken. Not a breaker event.

        Exists so callers have somewhere honest to route this outcome, rather than
        reaching for record_failure() because it is the nearest name.
        """
        return None

    # ---------------------------------------------------------------- reporting

    def describe(self) -> str:
        state = self.state
        if state is BreakerState.CLOSED:
            return "closed — local attempts allowed"
        if state is BreakerState.HALF_OPEN:
            return "half-open — one probe attempt allowed"
        remaining = max(0.0, self.cooldown_s - self._elapsed_since_open())
        return (
            f"open — all traffic to cloud, retrying local in {remaining:.0f}s "
            f"(after {self._consecutive_failures} consecutive infrastructure failures)"
        )
