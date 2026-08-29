"""Where the sidecar writes when there is no repository to write into.

The sidecar's log paths defaulted to `evals/traffic.jsonl` — correct for us, and the
single thing that made it un-installable by anyone else. A customer has no checkout, and
a relative default writes into whatever directory they happened to be standing in, or
fails.

So the default is a per-user data directory, chosen by platform convention:

    Windows   %LOCALAPPDATA%\\Kerna
    macOS     ~/Library/Application Support/Kerna
    Linux     $XDG_DATA_HOME/kerna, else ~/.local/share/kerna

## Why this is not merely tidiness

These files are the product's evidence. A path that silently resolves somewhere
different than the operator expects means a run that looks healthy and writes its rows
where nobody will look for them — the same failure shape as a cohort label that does not
describe the cohort. The resolved paths are therefore printed at startup, every time,
rather than documented.

## What is deliberately not here

No config file, no environment-variable indirection beyond the platform's own. One
override exists and it is the command-line flag, which is visible in the shell history
that produced the run.
"""

from __future__ import annotations

import os
import sys
from pathlib import Path

APP = "Kerna"


def data_dir() -> Path:
    """The per-user directory for evidence this install produces."""
    if sys.platform == "win32":
        base = os.environ.get("LOCALAPPDATA")
        root = Path(base) if base else Path.home() / "AppData" / "Local"
        return root / APP

    if sys.platform == "darwin":
        return Path.home() / "Library" / "Application Support" / APP

    base = os.environ.get("XDG_DATA_HOME")
    root = Path(base) if base else Path.home() / ".local" / "share"
    return root / APP.lower()


def default_log(name: str) -> Path:
    """Default path for one evidence file. Never creates anything."""
    return data_dir() / name


def ensure_parent(path: Path) -> Path:
    """Make a log's directory exist, returning the path.

    Failure is left to the caller's open(): a directory that cannot be created is a real
    problem the operator must see, and swallowing it here would produce a sidecar that
    runs happily and records nothing.
    """
    path.parent.mkdir(parents=True, exist_ok=True)
    return path
