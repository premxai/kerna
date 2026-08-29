#!/usr/bin/env python
"""Build `kerna-observe` as one self-contained binary.

Run from the repo root:

    .venv/Scripts/python.exe packaging/build.py

The output lands in `dist/`. It carries its own Python, so the machine it runs on needs
nothing installed -- which is the entire point and the only reason this file exists.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
# Intermediates go outside the repo. This tree lives under OneDrive, whose sync agent
# holds handles on files PyInstaller wants to delete -- the build then fails with
# WinError 5 and, worse, leaves the previous binary in dist/ so the next test run
# silently exercises stale code.
WORK = Path(tempfile.gettempdir()) / "kerna-observe-build"
EVALS = ROOT / "evals"
ENTRY = ROOT / "packaging" / "kerna_observe.py"
NAME = "kerna-observe"


def main() -> int:
    out = ROOT / "dist" / (NAME + (".exe" if sys.platform == "win32" else ""))
    # Removed before building, never after: a build that fails while an old binary
    # sits in dist/ is a test that passes against code nobody wrote today.
    out.unlink(missing_ok=True)

    if not ENTRY.exists():
        print(f"missing entry point: {ENTRY}", file=sys.stderr)
        return 1

    cmd = [
        sys.executable, "-m", "PyInstaller",
        "--onefile",
        "--name", NAME,
        "--paths", str(EVALS),
        "--console",
        "--noconfirm",
        "--clean",
        "--distpath", str(ROOT / "dist"),
        "--workpath", str(WORK),
        "--specpath", str(WORK),
        # The cascade package is reached through runtime imports in the subcommand
        # dispatch, so the analyser cannot see it by following imports alone.
        "--hidden-import", "m0.cascade.interceptor",
        "--hidden-import", "m0.cascade.dashboard",
        "--hidden-import", "m0.cascade.datadir",
        "--hidden-import", "m0.cascade.showcase",
        "--hidden-import", "m0.registry.models",
        "--hidden-import", "m0.registry.device",
        # Nothing here reads a corpus or validates a schema; the eval harness's
        # dependencies would triple the binary for code that never runs in it.
        "--exclude-module", "yaml",
        "--exclude-module", "pydantic",
        "--exclude-module", "pytest",
        "--exclude-module", "tkinter",
        str(ENTRY),
    ]

    print(" ".join(cmd), "\n")
    result = subprocess.run(cmd, cwd=ROOT)
    if result.returncode != 0:
        return result.returncode

    if not out.exists():
        print(f"build reported success but {out} is missing", file=sys.stderr)
        return 1

    print(f"\n{out}  ({out.stat().st_size / 1_048_576:.1f} MB)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
