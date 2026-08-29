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

    from m0.cascade.datadir import data_dir
    from m0.cascade.interceptor import DEFAULT_PORT

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

    from m0.registry.device import profile, render
    from m0.registry.models import candidates_for, refused

    ap = argparse.ArgumentParser(prog="kerna-observe models")
    ap.add_argument("--vram", type=float, default=None,
                    help="size for a different machine, in GB, instead of this one")
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
        from m0.cascade.showcase import main as demo_main

        return demo_main(rest)

    if verb == "install":
        return _install(rest)

    if verb == "run":
        from m0.cascade.interceptor import main as serve_main

        return serve_main(rest)

    if verb == "report":
        from m0.cascade.dashboard import main as report_main

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
