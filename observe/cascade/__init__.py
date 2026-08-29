"""The cascade sidecar — the first product code in this repository.

Three pieces, in dependency order:

  circuit.py      the breaker that makes "fail open" true under sustained failure
  dispatcher.py   the routing decision, which defaults to "not here"
  interceptor.py  the OpenAI-compatible endpoint that is safe to put in the path

It lives under `m0/` rather than a separate top-level package so it shares the harness's
imports, tests and conventions — the same measurement discipline applies to product code,
and splitting the tree would have quietly created a second standard.

**Local routing is off.** Every request forwards upstream, unchanged, until the C0a
fairness test has run. See `dispatcher.LOCAL_ROUTING_ENABLED` — the flag is a claim we
have not yet earned, written as code.
"""
