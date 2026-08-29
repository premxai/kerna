"""Wilson score interval, with no dependencies and nothing to render.

INV-13: every rate carries its bound. A perfect 20-item run does not pass an 85% bar, and
the same discipline gates shadow-to-live promotion.

## Why it lives on its own

It was defined inside `dashboard.py`, so anything wanting an interval had to import the
HTML renderer. The replay CLI did exactly that, for one function, and inherited a
SyntaxError in some markup — which surfaced only after a completed run, on the line that
was about to print its results.

`m0.metrics` has the same function and imports pydantic for the eval schema, which is the
other direction of the same mistake: the product does not import the research harness.

Interval arithmetic depends on neither. It is four lines of `math` and belongs where
anything can reach it without consequence.
"""

from __future__ import annotations

import math

Z_95 = 1.96


def wilson(successes: int, total: int, z: float = Z_95) -> tuple[float, float]:
    """(low, high) for a binomial proportion.

    Zero trials returns (0, 0) rather than raising: a caller with no data should render a
    dash, and making it handle an exception invites a bare `except` that hides real ones.
    """
    if total <= 0:
        return 0.0, 0.0
    p = successes / total
    d = 1 + z * z / total
    centre = (p + z * z / (2 * total)) / d
    half = z * math.sqrt(p * (1 - p) / total + z * z / (4 * total * total)) / d
    return max(0.0, centre - half), min(1.0, centre + half)


def wilson_lower_bound(successes: int, total: int, z: float = Z_95) -> float:
    return wilson(successes, total, z)[0]
