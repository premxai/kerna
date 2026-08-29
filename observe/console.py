"""Printing that survives the terminal it lands in.

Two tools have now crashed at the last step for the same reason: the default Windows
console is cp1252, our reports contain `≥` and `—` and repository descriptions are full
of emoji, and `print()` raises `UnicodeEncodeError` **after** all the work is done. The
landscape scout lost a completed scan to it; the checkable-slice measurement lost a
completed measurement to it, on a capture the operator had spent twenty minutes producing.

Fixing it in each tool as it appears is how a bug class becomes permanent, so it lives
here once.

## Why not simply sanitise the string

The scout's first fix replaced unencodable characters with `?`, which works and quietly
makes every report worse on terminals that would have rendered them perfectly. Modern
Windows Terminal handles UTF-8 fine; it is only the *default stream encoding* that does
not. So the order is: switch the stream to UTF-8 if it will accept it, and only degrade
characters when that fails.

A report is the product of these tools. Losing one to a code page is a worse outcome than
any character it contains.
"""

from __future__ import annotations

import sys
from typing import Any, TextIO


def enable_utf8(stream: TextIO | None = None) -> bool:
    """Switch a stream to UTF-8 if it can be. Returns whether it worked.

    `reconfigure` exists on 3.7+ but not on every stream object — a pytest capture, a
    pipe wrapper or a redirect may not have it, which is why this is a request rather
    than an instruction.
    """
    stream = stream or sys.stdout
    reconfigure = getattr(stream, "reconfigure", None)
    if reconfigure is None:
        return False
    try:
        reconfigure(encoding="utf-8")
        return True
    except (ValueError, OSError, AttributeError):
        return False


def console_safe(text: str, stream: TextIO | None = None) -> str:
    """Make a string printable on whatever encoding the stream actually has."""
    stream = stream or sys.stdout
    encoding = getattr(stream, "encoding", None) or "utf-8"
    try:
        text.encode(encoding)
        return text
    except (UnicodeEncodeError, LookupError):
        return text.encode(encoding, errors="replace").decode(encoding, errors="replace")


def emit(text: str = "", stream: TextIO | None = None) -> None:
    """Print without ever raising on the encoding.

    Tries UTF-8 first so a capable terminal shows the real characters, and degrades only
    when the stream refuses. The one thing it will not do is fail.
    """
    stream = stream or sys.stdout
    enable_utf8(stream)
    try:
        print(text, file=stream)
    except UnicodeEncodeError:
        print(console_safe(text, stream), file=stream)
