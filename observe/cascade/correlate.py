"""One id per turn, so three logs describe the same thing.

Three components write rows about a single agent turn and none of them can currently be
joined to the others:

  * `recorder.py` — what was sent, what it cost
  * `enforce.py` — what the agent tried to do and whether policy allowed it
  * `explore.py` — whether the local model would have agreed

Without a shared key, "this expensive turn was also the one we blocked, and the local model
would have got it right" is a sentence nobody can produce. That sentence is the product.

## Whoever sees the turn first owns its identity

The id is **inherited if present and minted if not.** The outermost process in the chain
defines identity and everything downstream agrees, which is what makes a cross-process join
work without any coordination beyond a header.

That matters because of Decision 041's architecture: when Kerna's runtime calls through
this sidecar, Kerna already has a task id in its own SQLite audit trail. If it sends that id
on the request, our rows and its rows carry the same key and the two halves of the product
share one timeline. If it sends nothing, we mint one and remain internally consistent.
**Nothing breaks either way**, which is the property that lets the halves ship separately.

## What it is not

Not a user id, not a session id, not derived from content. An opaque random token scoped to
one turn, carrying no information about who or what. Decision 005 governs what may leave
the device and this deliberately adds nothing to that surface — it is a join key, and a
join key that describes its subject is a leak waiting for someone to notice it.
"""

from __future__ import annotations

import uuid
from typing import Any

# Emitted on our responses so a client can correlate its own logs with ours.
TURN_HEADER = "x-cascade-turn"

# Inbound headers we will adopt, most specific first. `x-kerna-task` is the one that makes
# the cross-process join work; the others are conventions worth honouring when a customer's
# infrastructure already sets them.
INHERITED_HEADERS: tuple[str, ...] = (
    TURN_HEADER,
    "x-kerna-task",
    "x-kerna-task-id",
    "x-request-id",
    "x-correlation-id",
)

_MAX_INHERITED = 128


def mint() -> str:
    """A fresh turn id. Short enough to read in a log, wide enough not to collide."""
    return f"t_{uuid.uuid4().hex[:16]}"


def _clean(value: str) -> str | None:
    """An inherited id must be safe to write into a log line and read back.

    Anything a caller can put in a header eventually appears in a JSONL file and a report,
    so a control character or a newline here is a log-injection bug rather than an
    inconvenience. Rejecting is safer than sanitising: a rejected id is replaced by one we
    minted, which is always valid.
    """
    value = value.strip()
    if not value or len(value) > _MAX_INHERITED:
        return None
    if any(ch.isspace() or ord(ch) < 32 for ch in value):
        return None
    return value


def turn_id(headers: dict[str, Any] | None) -> str:
    """The id for this turn: inherited when a caller supplied one, minted otherwise."""
    if headers:
        lowered = {str(k).lower(): v for k, v in headers.items()}
        for name in INHERITED_HEADERS:
            raw = lowered.get(name)
            if isinstance(raw, str):
                cleaned = _clean(raw)
                if cleaned:
                    return cleaned
    return mint()


def was_inherited(headers: dict[str, Any] | None, resolved: str) -> bool:
    """Whether this id came from upstream. Worth recording: a run where every id was
    minted means the cross-process join is not actually happening, and that looks
    identical to one where it is, until someone tries to join the logs."""
    if not headers:
        return False
    lowered = {str(k).lower(): v for k, v in headers.items()}
    return any(_clean(str(lowered[n])) == resolved
               for n in INHERITED_HEADERS if isinstance(lowered.get(n), str))
