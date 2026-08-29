"""`kerna-observe` — the shadow assessment, as one binary.

## What this is for

The sidecar has always worked. It was never *installable*: it needed Python, a virtual
environment, a repository checkout, and log paths that only resolve inside that checkout.
That is fine for the people who wrote it and disqualifying for everyone else — "and
you'll need Python 3.12" is a support conversation before anything has been demonstrated.

This is the same code with the prerequisites removed. One file, no runtime to install,
no checkout, and evidence written to a per-user data directory (see `datadir.py`).

## Why it carries the Kerna name

Two binaries is a build fact. Two *products* would be a strategic error — two names, two
docs, two install pages, and a company selling neither. Decision 044 settled that there
is one name; this is that decision expressed at the install surface. A user should never
have to learn there are two artifacts unless they read a process list.

## Four verbs: three steps, and a way to see all three at once

    kerna-observe demo        stand the whole system up and open the report
    kerna-observe install     what to set, and what to unset to remove it
    kerna-observe run         sit in the path; serve nothing locally
    kerna-observe report      turn the evidence into one HTML page

`demo` needs no API key, no provider account, no local model and no network. It is the
answer to "show me" in a room, which the system could do for nobody until it existed.

`install` prints rather than does. Editing a customer's shell profile to place ourselves
in the path of their production traffic is not a thing to do on their behalf, and the
one-variable uninstall is the property that gets this accepted in the first place — it
stops being credible the moment we start writing to their dotfiles.
"""

from __future__ import annotations

import sys
from pathlib import Path
from typing import Any

# Running this file directly puts *its own* directory on sys.path, not the repository
# root, so `import observe` fails from a fresh clone. The packaged binary never hits
# this because PyInstaller is told the package root explicitly -- which meant the
# script path worked for nobody who had not already built a binary, and the first
# thing a person does with a checkout is run the file.
if __package__ in (None, ""):
    _root = Path(__file__).resolve().parents[2]
    if str(_root) not in sys.path:
        sys.path.insert(0, str(_root))

USAGE = """kerna-observe: measure what your coding agents cost, and what could run locally.

  kerna-observe demo                    stand the whole thing up and open the report
  kerna-observe models                  what this machine can run, and what it cannot
  kerna-observe install [--port N]      show how to put it in the path, and how to remove it
  kerna-observe run --upstream URL      run the sidecar (serves nothing locally)
  kerna-observe report [--out FILE]     write the HTML report

Run a subcommand with --help for its options.
"""


def _install(argv: list[str]) -> int:
    import argparse

    from observe.cascade.datadir import data_dir
    from observe.cascade.interceptor import DEFAULT_PORT

    ap = argparse.ArgumentParser(prog="kerna-observe install")
    ap.add_argument("--port", type=int, default=DEFAULT_PORT)
    args = ap.parse_args(argv)

    base = f"http://127.0.0.1:{args.port}/v1"
    print("Point your agent at the sidecar:\n")
    print(f"    OPENAI_BASE_URL={base}")
    print(f"    ANTHROPIC_BASE_URL=http://127.0.0.1:{args.port}\n")
    print("To remove it, unset those. There is nothing else to uninstall.\n")
    print(f"Evidence is written to: {data_dir()}")
    print("Your provider key is passed through untouched and never stored.")
    return 0


def _models(argv: list[str]) -> int:
    """What this machine can actually run.

    The registry, the licence gate and the device tiering have existed since M0 and have
    never been reachable from the product -- a capability nobody can invoke is one that
    might as well not be built. Sizing is on *free* VRAM rather than total, because
    sizing on total over-promises on every machine that is already running something.
    """
    import argparse

    from observe.registry.device import profile, render
    from observe.registry.models import candidates_for, refused

    ap = argparse.ArgumentParser(prog="kerna-observe models")
    ap.add_argument("--vram", type=float, default=None,
                    help="size for a different machine, in GB, instead of this one")
    ap.add_argument("--download", action="store_true",
                    help="fetch the largest model that fits without asking")
    ap.add_argument("--no-download", action="store_true",
                    help="never fetch, and do not ask")
    args = ap.parse_args(argv)

    if args.vram is None:
        device = profile()
        print(render(device))
        free = device.free_vram_gb
    else:
        free = args.vram
        print(f"sizing for {free:.1f} GB of free VRAM (not this machine)")

    fits = candidates_for(free)
    print()
    print("coding models that fit:")
    if fits:
        for model in fits:
            print(f"  {model.name}")
    else:
        print("  none. Local routing is not available on this machine.")

    blocked = refused()
    if blocked:
        # Named rather than hidden. A model missing with no reason given looks like a
        # gap in the catalogue instead of a decision.
        print()
        print("excluded on licence, not on capability:")
        for model in blocked:
            print(f"  {model.name}")

    print()
    print("Fitting is necessary and not sufficient: whether a model may be routed to")
    print("is earned per task class from measured agreement, never assumed from size.")

    if args.vram is not None or not fits:
        # Nothing to offer: either this is a what-if for another machine, or no
        # model fits here.
        return 0
    return _offer_download(fits[0], download=args.download, skip=args.no_download)


def _offer_download(card: Any, *, download: bool, skip: bool) -> int:
    """Ask once whether to fetch the largest model that fits.

    Asking beats doing. These files are gigabytes on someone's metered laptop,
    and a tool that starts a download because it decided the machine could take
    one is a tool people uninstall. `--download` and `--no-download` exist so
    the same command is scriptable and so CI never blocks on a prompt.
    """
    import os
    from pathlib import Path

    from observe.registry.installer import download as fetch

    if not card.downloadable:
        print()
        print(f"{card.name} fits, but has no verified download; refusing to guess a URL.")
        return 0

    # Same environment variable the standalone installer honours, so one machine
    # does not end up with two copies of the same weights.
    destination = Path(os.environ.get("LOCALM_MODELS", "models"))
    existing = destination / card.hf_file
    if existing.is_file():
        print()
        print(f"already downloaded: {existing}")
        return 0

    print()
    print(f"largest model that fits: {card.name}  ({card.file_size_gb:.1f} GB download)")
    print(f"destination: {destination.resolve()}")

    if skip:
        print("skipped (--no-download). Re-run without it when you want the model.")
        return 0
    if not download:
        if not sys.stdin.isatty():
            # Not a terminal: a prompt nobody can answer would hang a pipe.
            print("Run with --download to fetch it, or --no-download to silence this.")
            return 0
        try:
            answer = input("Download it now? [y/N] ").strip().lower()
        except (EOFError, KeyboardInterrupt):
            print()
            return 0
        if answer not in ("y", "yes"):
            print("Skipped. Nothing was downloaded; re-run when you want it.")
            return 0

    try:
        path = fetch(card, destination)
    except Exception as exc:  # noqa: BLE001
        # A failed download is not a failed machine check. The report above it
        # is still true and still worth having.
        print(f"download failed: {exc}")
        return 1
    print(f"downloaded: {path}")
    return 0


def _make_output_unbreakable() -> None:
    """Never let a character kill the process.

    A Windows console is cp1252 by default, and the banner contains a middle dot. Printing
    it raised `UnicodeEncodeError` and took the whole sidecar down -- twice in this
    project's history, once destroying a completed run's report before it was written.

    `errors="replace"` is the important half: UTF-8 first, and where the terminal cannot
    represent a character it gets a placeholder instead of an exception. A monitoring tool
    that dies while describing itself is worse than one that prints a question mark.
    """
    for stream in (sys.stdout, sys.stderr):
        try:
            # line_buffering matters as much as the encoding. This runs for hours with
            # its output redirected to a file, where Python's default block buffering
            # means the operator sees an empty log -- including the startup banner and
            # including any error explaining why it stopped. That is how a bind failure
            # stayed invisible long enough to send every test request to the wrong
            # server.
            stream.reconfigure(encoding="utf-8", errors="replace", line_buffering=True)
        except Exception:  # noqa: BLE001
            pass


def main(argv: list[str] | None = None) -> int:
    _make_output_unbreakable()
    argv = list(sys.argv[1:] if argv is None else argv)

    if not argv or argv[0] in ("-h", "--help", "help"):
        print(USAGE)
        return 0

    verb, rest = argv[0], argv[1:]

    if verb == "models":
        return _models(rest)

    if verb == "demo":
        from observe.cascade.showcase import main as demo_main

        return demo_main(rest)

    if verb == "install":
        return _install(rest)

    if verb == "run":
        from observe.cascade.interceptor import main as serve_main

        return serve_main(rest)

    if verb == "report":
        from observe.cascade.dashboard import main as report_main

        return report_main(rest)

    if verb == "--version":
        print(VERSION)
        return 0

    print(f"unknown command: {verb}\n", file=sys.stderr)
    print(USAGE, file=sys.stderr)
    return 2


VERSION = "0.1.0"


if __name__ == "__main__":
    raise SystemExit(main())
