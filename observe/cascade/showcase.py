"""`kerna-observe demo` — the whole product in one command, on any machine.

Everything the system does has been provable for a while and demonstrable by nobody: it
took a stub provider, two processes, a config edit and a SQLite query. That is a test,
not something you can show in a room.

This stands the whole thing up, drives three agent turns through it, and opens the
report. No API key, no provider account, no local model, no network.

## Three scenarios: two governance rungs, then the saving

The demo used to run only the first of these, and its report was a row of dashes:
"Blocked by policy —", "the gate has not run". That is an accurate picture of rung 1 and
a poor demonstration of the half this project would sell.

  1. **observe** — a blocking turn through the ordinary sidecar. Spend is recorded, the
     runtime's policy decision is recorded, and *nothing is enforced*. This is the honest
     install default (040): it cannot hurt a request because it does not touch one.
  2. **enforce** — a streaming turn through a sidecar whose gate is in `enforce` with a
     policy denying `Write`. The agent asks to rewrite a config file; the client receives
     `[blocked by policy]` in place of the tool call, keeps the prose that preceded it,
     and the turn ends without `stop_reason: tool_use` so nothing waits for a result it
     will never be asked to give.

  3. **route** — a sidecar with a promoted class and the fairness gate open. An eligible
     turn is answered on the laptop, the cloud is never asked, and the response reports
     zero tokens. That last number is the one the product exists to move.

The gate is a **stream** filter — it holds a tool block until the policy has ruled on it
— so a blocking request can never exercise it. That, not a missing feature, is why the
gate had never run in this demo despite being built and tested.

## The routing ledger is synthetic, and that is not a detail

A ledger records agreement with the cloud, on this machine, with this model. It is the
thing that authorises serving a customer a local answer, and filling one from anywhere
else would make routing work in a room while lying about what earned it — the failure
this project has caught repeatedly in its own numbers.

So the demo's ledger says `synthetic` in its own cohort keys, and it demonstrates the
**wiring**: promotion reaches the dispatcher, the gate opens, a request is served without
the cloud. It demonstrates nothing about whether a real model should be trusted with that
class. `qualify.py` earns that against a real model, and its evidence never mixes with
this one.

## What it is honest about

A demo that looks like a customer's real data is the same error this project keeps
finding in its own measurements, so the report is labelled as synthetic in its title and
in a banner, and the numbers come from a stub whose token counts are fixed constants.
Nothing here should ever be quoted.

## Why the sidecar runs in-process

Under PyInstaller `sys.executable` is the packaged binary, not a Python interpreter, so
spawning `python -m observe.cascade.interceptor` works from a source checkout and fails in the
artifact people actually download. The demo builds the same handler the real command
builds and serves it on a thread.

## The Kerna half is optional and says so

Governance needs the `kerna` binary. When it is absent the demo runs anyway and the
report's governance panel reports that no audit trail was supplied — which is exactly
what it should say, and is more useful than refusing to start.
"""

from __future__ import annotations

import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any

from .circuit import CircuitBreaker
from .dispatcher import Dispatcher
from .interceptor import build_handler, make_cloud_forwarder
from .recorder import TrafficRecorder

# Fixed, so two runs of the demo produce the same page. Chosen to be recognisably
# round rather than plausible -- nobody should mistake these for a measurement.
STUB_PROMPT_TOKENS = 12_000
STUB_CACHED_TOKENS = 30_000
STUB_OUTPUT_TOKENS = 300

DEMO_TOOL = "network_probe"
DEMO_GOAL = "check whether the build server is reachable"


def _free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def _find_kerna() -> str | None:
    explicit = os.environ.get("KERNA_BIN")
    if explicit and Path(explicit).is_file():
        return explicit
    found = shutil.which("kerna")
    if found:
        return found
    for candidate in (
        Path.home() / "kerna" / "target" / "debug" / "kerna.exe",
        Path.home() / "kerna" / "target" / "debug" / "kerna",
    ):
        if candidate.is_file():
            return str(candidate)
    return None


DENIED_TOOL = "Write"
DENIED_PATH = "src/config/production.yaml"
DENIAL_REASON = "the agent may read this repository, not rewrite its config"


def _sse(event: str, payload: dict[str, Any]) -> bytes:
    return f"event: {event}\ndata: {json.dumps(payload)}\n\n".encode()


def _write_turn() -> list[bytes]:
    """The Anthropic SSE frames for: a sentence, then a `Write`.

    Written out rather than imported from the tests, because a demo that shares a fixture
    with its test can pass while the shipped path is broken.
    """
    return [
        _sse("message_start", {"type": "message_start", "message": {"id": "msg_demo"}}),
        _sse("content_block_start", {"type": "content_block_start", "index": 0,
                                     "content_block": {"type": "text", "text": ""}}),
        _sse("content_block_delta", {"type": "content_block_delta", "index": 0,
                                     "delta": {"type": "text_delta",
                                               "text": "I'll update the config."}}),
        _sse("content_block_stop", {"type": "content_block_stop", "index": 0}),
        _sse("content_block_start", {"type": "content_block_start", "index": 1,
                                     "content_block": {"type": "tool_use", "id": "tu_1",
                                                       "name": DENIED_TOOL, "input": {}}}),
        _sse("content_block_delta", {"type": "content_block_delta", "index": 1,
                                     "delta": {"type": "input_json_delta",
                                               "partial_json": json.dumps(
                                                   {"file_path": DENIED_PATH,
                                                    "content": "replicas: 0\n"})}}),
        _sse("content_block_stop", {"type": "content_block_stop", "index": 1}),
        _sse("message_delta", {"type": "message_delta",
                               "delta": {"stop_reason": "tool_use"}}),
        _sse("message_stop", {"type": "message_stop"}),
    ]


class _StubProvider:
    """Stands in for the customer's provider.

    Answers the first request with a tool call and the second with text, which is the
    smallest exchange that exercises a policy decision and terminates.
    """

    def __init__(self) -> None:
        self.port = _free_port()
        self.calls = 0
        demo = self

        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def log_message(self, *args: Any) -> None:
                return

            def do_POST(self) -> None:  # noqa: N802
                raw = self.rfile.read(int(self.headers.get("Content-Length") or 0))

                # The enforced scenario needs an Anthropic SSE turn, because the gate is
                # a stream filter -- it holds a tool block until the policy has ruled on
                # it, which a single blocking JSON body gives it no opportunity to do.
                # That is also why the demo showed "the gate has not run" for as long as
                # every request it made was blocking.
                try:
                    asked = json.loads(raw or b"{}")
                except json.JSONDecodeError:
                    asked = {}
                if asked.get("stream"):
                    self._stream_a_write()
                    return

                demo.calls += 1

                if demo.calls == 1:
                    message = {
                        "role": "assistant",
                        "content": None,
                        "tool_calls": [{
                            "id": "call_demo",
                            "type": "function",
                            "function": {"name": DEMO_TOOL, "arguments": "{}"},
                        }],
                    }
                    finish = "tool_calls"
                else:
                    message = {"role": "assistant",
                               "content": "I could not reach it."}
                    finish = "stop"

                body = json.dumps({
                    "id": "chatcmpl-demo",
                    "object": "chat.completion",
                    "model": "claude-opus-5",
                    "choices": [{"index": 0, "finish_reason": finish,
                                 "message": message}],
                    "usage": {
                        "prompt_tokens": STUB_PROMPT_TOKENS,
                        "completion_tokens": STUB_OUTPUT_TOKENS,
                        "input_tokens": STUB_PROMPT_TOKENS,
                        "output_tokens": STUB_OUTPUT_TOKENS,
                        "cache_read_input_tokens": STUB_CACHED_TOKENS,
                    },
                }).encode()

                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def _stream_a_write(self) -> None:
                """One Anthropic turn: a sentence, then a `Write` the policy forbids.

                `Connection: close` because an SSE body under HTTP/1.1 has no
                Content-Length and this stub does not chunk -- without it the client
                waits for a length that never comes and the turn times out.
                """
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.send_header("Cache-Control", "no-cache")
                self.send_header("Connection", "close")
                self.end_headers()
                for frame in _write_turn():
                    self.wfile.write(frame)
                    self.wfile.flush()
                self.close_connection = True

        self.server = ThreadingHTTPServer(("127.0.0.1", self.port), Handler)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def close(self) -> None:
        self.server.shutdown()
        self.server.server_close()


class _Sidecar:
    """The real handler, served on a thread. Same code path as `kerna-observe run`."""

    def __init__(self, upstream: str, traffic_log: Path) -> None:
        self.port = _free_port()
        self.recorder = TrafficRecorder(path=traffic_log)

        handler = build_handler(
            dispatcher=Dispatcher(breaker=CircuitBreaker(), idle=False),
            forward_cloud=make_cloud_forwarder(upstream),
            attempt_local=None,
            upstream=upstream,
            recorder=self.recorder,
            quiet=True,
        )

        class Exclusive(ThreadingHTTPServer):
            # Never share a port. On Windows the default would let a second sidecar bind
            # the same address, and the two would split the evidence silently.
            allow_reuse_address = False

        self.server = Exclusive(("127.0.0.1", self.port), handler)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def close(self) -> None:
        self.server.shutdown()
        self.server.server_close()


LOCAL_ANSWER_TEXT = "answered on this laptop (demo)"


class _RoutingSidecar:
    """A sidecar with a promoted class and the fairness gate open, so a request is
    actually served locally and the cloud never sees it.

    The ledger here is **synthetic and labelled as such**, and that distinction is the
    whole reason this class exists rather than a fixture written into the operator's real
    data directory. A ledger records agreement with the cloud, on this machine, with this
    model -- it is the thing that authorises serving a customer a local answer. Filling
    one from anywhere else would make routing work in a room while being a lie about what
    earned it, which is the failure this project has caught repeatedly.

    So this proves the *wiring*: promotion reaches the dispatcher, the gate opens, and a
    request is answered without the cloud. It proves nothing about whether a real local
    model should be trusted with that class. `qualify.py` is what earns that, against a
    real model, and its evidence never mixes with this.
    """

    def __init__(self, upstream: str, traffic_log: Path, ledger_path: Path) -> None:
        from .ledger import MIN_TRIALS_TO_PROMOTE, Cohort, Ledger

        self.port = _free_port()
        self.recorder = TrafficRecorder(path=traffic_log)
        self.served_locally = 0

        # Marked in the cohort itself, so a stray copy of this file can never be mistaken
        # for evidence: the model name says what it is.
        ledger = Ledger(path=ledger_path)
        cohort = Cohort(task_class="search", machine_tier="demo (synthetic)",
                        model="demo-local-model (synthetic)")
        for _ in range(MIN_TRIALS_TO_PROMOTE + 5):
            ledger.observe(cohort, True)
        ledger.save()
        promoted = frozenset({"search"})

        def attempt_local(payload: dict[str, Any]) -> dict[str, Any]:
            self.served_locally += 1
            return {
                "id": "chatcmpl-local-demo",
                "object": "chat.completion",
                "model": "demo-local-model",
                "choices": [{"index": 0, "finish_reason": "stop", "message": {
                    "role": "assistant", "content": LOCAL_ANSWER_TEXT}}],
                # Zero, because nothing was bought. This is the number the whole
                # product exists to move.
                "usage": {"prompt_tokens": 0, "completion_tokens": 0,
                          "input_tokens": 0, "output_tokens": 0},
            }

        handler = build_handler(
            dispatcher=Dispatcher(breaker=CircuitBreaker(), idle=False,
                                  local_routing_enabled=True,
                                  promoted_classes=promoted),
            forward_cloud=make_cloud_forwarder(upstream),
            attempt_local=attempt_local,
            upstream=upstream,
            recorder=self.recorder,
            quiet=True,
        )

        class Exclusive(ThreadingHTTPServer):
            allow_reuse_address = False

        self.server = Exclusive(("127.0.0.1", self.port), handler)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def close(self) -> None:
        self.server.shutdown()
        self.server.server_close()


def _drive_routed_turn(port: int) -> dict[str, Any]:
    """One agentic turn that a promoted class should answer locally.

    Carries tool definitions **and** a prior tool result, because that is what makes a
    turn eligible (036): the loop is already running and nobody is waiting on a first
    reply. A turn without both would be forwarded to the cloud and the demo would show
    the opposite of what it claims.
    """
    import httpx

    response = httpx.post(
        f"http://127.0.0.1:{port}/v1/chat/completions",
        json={"model": "claude-opus-5",
              "tools": [{"type": "function",
                         "function": {"name": "Grep", "parameters": {}}}],
              "messages": [
                  {"role": "user", "content": "where is the retry logic?"},
                  {"role": "assistant", "content": None, "tool_calls": [{
                      "id": "c1", "type": "function",
                      "function": {"name": "Grep", "arguments": "{}"}}]},
                  {"role": "tool", "tool_call_id": "c1", "content": "3 matches"},
              ]},
        timeout=30,
    )
    return response.json()


class _EnforcingSidecar:
    """The same handler, with the gate in `enforce` and a policy that denies `Write`.

    Kept separate from the observing sidecar on purpose. The demo's first scenario is
    rung 1 -- audit only, denies nothing, which is the honest *install* default (040) --
    and showing both side by side is the point: the same seam, one rung apart, and the
    difference is whether the client gets the tool call or the refusal.
    """

    def __init__(self, upstream: str, traffic_log: Path, gate_log: Path) -> None:
        from .enforce import Decision, Policy, Rule

        self.port = _free_port()
        self.recorder = TrafficRecorder(path=traffic_log)
        self.gate_log = gate_log
        self.rows: list[dict[str, Any]] = []

        def record(row: dict[str, Any]) -> None:
            self.rows.append(row)
            with self.gate_log.open("a", encoding="utf-8") as handle:
                handle.write(json.dumps(row, ensure_ascii=False) + "\n")

        handler = build_handler(
            dispatcher=Dispatcher(breaker=CircuitBreaker(), idle=False),
            forward_cloud=make_cloud_forwarder(upstream),
            attempt_local=None,
            upstream=upstream,
            recorder=self.recorder,
            gate_policy=Policy(rules=(Rule(DENIED_TOOL, Decision.DENY, DENIAL_REASON),)),
            gate_mode="enforce",
            record_gate=record,
            quiet=True,
        )

        class Exclusive(ThreadingHTTPServer):
            allow_reuse_address = False

        self.server = Exclusive(("127.0.0.1", self.port), handler)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def close(self) -> None:
        self.server.shutdown()
        self.server.server_close()


def _drive_enforced_turn(port: int) -> str:
    """One streaming turn through the enforcing sidecar; returns what the client got."""
    import httpx

    with httpx.stream(
        "POST", f"http://127.0.0.1:{port}/v1/messages",
        json={"model": "claude-opus-5", "stream": True, "max_tokens": 256,
              "tools": [{"name": DENIED_TOOL, "input_schema": {"type": "object"}}],
              "messages": [{"role": "user", "content": DEMO_GOAL}]},
        timeout=30,
    ) as response:
        return "".join(response.iter_text())


def _write_kerna_config(workspace: Path, sidecar_port: int) -> None:
    config = workspace / "kerna.toml"
    existing = [
        line for line in config.read_text(encoding="utf-8").splitlines()
        if not line.startswith(("llm_provider", "llm_model", "llm_api_key"))
    ]
    # Top-level keys must precede the first table, or TOML nests them inside whichever
    # table came last and the parser then reports them missing rather than misplaced.
    head = [
        'llm_provider = "kernaobserve"',
        'llm_model = "claude-opus-5"',
        'llm_api_key = "demo-key-not-used"',
    ]
    tail = [
        "",
        "[providers.kernaobserve]",
        'type = "openai_compatible"',
        'default_model = "claude-opus-5"',
        f'base_url = "http://127.0.0.1:{sidecar_port}/v1"',
    ]
    config.write_text("\n".join(head + existing + tail) + "\n", encoding="utf-8")


def _run_kerna(kerna: str, workspace: Path, sidecar_port: int) -> tuple[bool, str]:
    """One governed turn through the sidecar. Returns (ran, note)."""
    init = subprocess.run(
        [kerna, "init"], cwd=workspace, capture_output=True,
        text=True, encoding="utf-8", errors="replace", timeout=180,
    )
    if init.returncode != 0:
        return False, f"`kerna init` failed: {init.stderr.strip()[:200]}"

    _write_kerna_config(workspace, sidecar_port)

    # Audit mode on purpose: rung 1 is what a customer's first day looks like, and it
    # gives the report a decision that was recorded and deliberately not enforced --
    # which is the distinction the governance panel exists to draw.
    result = subprocess.run(
        [kerna, "run", "--audit", DEMO_GOAL], cwd=workspace, capture_output=True,
        text=True, encoding="utf-8", errors="replace", timeout=240,
    )
    if result.returncode != 0:
        return False, f"`kerna run` failed: {result.stderr.strip()[:200]}"
    return True, ""


def main(argv: list[str] | None = None) -> int:
    import argparse
    import webbrowser

    from .dashboard import gather, kerna_budgets, kerna_events, read_jsonl, render

    parser = argparse.ArgumentParser(
        prog="kerna-observe demo",
        description="Stand up the whole system and open the report. No API key needed.",
    )
    parser.add_argument("--no-open", action="store_true",
                        help="write the report but do not open a browser")
    parser.add_argument("--out", type=Path, default=None,
                        help="where to write the report (default: a temporary directory)")
    args = parser.parse_args(argv)

    workdir = Path(tempfile.mkdtemp(prefix="kerna-demo-"))
    traffic_log = workdir / "traffic.jsonl"
    report = args.out or (workdir / "report.html")

    # ASCII on purpose. The packaged binary reconfigures stdout to UTF-8, but this same
    # module runs from a source checkout where it does not, and the first line a person
    # sees should not depend on which of those they used.
    print("kerna-observe demo: everything below is synthetic. "
          "Nothing here is a result.\n")

    provider = _StubProvider()
    sidecar = _Sidecar(f"http://127.0.0.1:{provider.port}/v1", traffic_log)
    print(f"  stub provider   127.0.0.1:{provider.port}")
    print(f"  sidecar         127.0.0.1:{sidecar.port}")

    kerna_db: Path | None = None
    note = ""

    try:
        kerna = _find_kerna()
        if kerna is None:
            note = ("The `kerna` binary was not found, so the governance half was "
                    "skipped. Set KERNA_BIN or put `kerna` on PATH.")
            print("  runtime         not found - governance half skipped")
            # Still drive one request through the sidecar, so the cost half is real.
            import httpx

            httpx.post(
                f"http://127.0.0.1:{sidecar.port}/v1/chat/completions",
                json={"model": "claude-opus-5",
                      "messages": [{"role": "user", "content": DEMO_GOAL}]},
                timeout=30,
            )
        else:
            print(f"  runtime         {kerna}")
            workspace = workdir / "workspace"
            workspace.mkdir()
            ran, failure = _run_kerna(kerna, workspace, sidecar.port)
            if ran:
                kerna_db = workspace / "kerna.db"
            else:
                note = failure
                print(f"  !! {failure}")

        time.sleep(1.0)          # the recorder appends per row
    finally:
        sidecar.close()

    # The scenario the demo was missing. Everything above is rung 1: it observes and
    # enforces nothing, which is honest and shows a report of dashes. This is the same
    # seam one rung up.
    gate_log = workdir / "gate.jsonl"
    enforcing = _EnforcingSidecar(
        f"http://127.0.0.1:{provider.port}/v1", traffic_log, gate_log)
    print(f"  enforcing       127.0.0.1:{enforcing.port}")
    try:
        received = _drive_enforced_turn(enforcing.port)
    finally:
        enforcing.close()
        provider.close()

    # Checked against the stream the client actually received, not against our own
    # bookkeeping. The gate replaces the tool block with a text block that *names* the
    # tool, so "Write is absent" is the wrong test and passes only by accident -- the
    # right one is that no `tool_use` survived and the refusal is there in its place.
    denied = [r for r in enforcing.rows
              if r.get("decision") == "deny" and r.get("enforced") is True]
    suppressed = "tool_use" not in received
    explained = "[blocked by policy]" in received
    clean_end = '"stop_reason": "tool_use"' not in received

    print()
    print(f"  the agent asked to {DENIED_TOOL} {DENIED_PATH}")
    if denied and suppressed and explained and clean_end:
        # Pulled apart rather than inlined into the f-string: an escape inside an
        # f-string *expression* is a SyntaxError before Python 3.12, so this parses on
        # the interpreter running it and fails on a customer's 3.11. The compat test
        # catches it; writing it this way means it never has to.
        line = next((l for l in received.splitlines() if "[blocked by policy]" in l), "")
        marker = '"text": "'
        shown = line.split(marker)[-1].rstrip('"}')[:96] if marker in line else line[:96]
        print(f"  the client received  {shown}")
        print("  no tool_use block survived, and the turn still ended cleanly -- so the")
        print("  agent is not left waiting for a result it will never be asked to give")
    else:
        # Never claim a denial that did not happen. A demo reporting protection it did
        # not deliver is the failure mode this project keeps finding in its own numbers.
        print(f"  !! not a clean denial (denied={bool(denied)} suppressed={suppressed} "
              f"explained={explained} clean_end={clean_end}) - do not present this run")

    # Scenario 3: a promoted class, the gate open, and the cloud never asked.
    routing = _RoutingSidecar(f"http://127.0.0.1:{provider.port}/v1",
                              traffic_log, workdir / "demo-ledger.json")
    print(f"  routing         127.0.0.1:{routing.port}")
    try:
        routed = _drive_routed_turn(routing.port)
    finally:
        routing.close()

    answer = (((routed.get("choices") or [{}])[0]).get("message") or {}).get("content")
    print()
    if routing.served_locally and answer == LOCAL_ANSWER_TEXT:
        print("  a promoted class was answered on the laptop, and the cloud was never")
        print(f"  asked: \"{answer}\"  (0 tokens bought)")
        print("  the ledger behind that is SYNTHETIC and labelled so in the file --")
        print("  it proves the wiring, not that any model earned the right")
    else:
        print("  !! the routed turn was not served locally - do not present this run")

    data = gather(
        read_jsonl(traffic_log),
        read_jsonl(gate_log),
        {},
        governance=kerna_events(kerna_db),
        budgets=kerna_budgets(kerna_db),
    )
    report.parent.mkdir(parents=True, exist_ok=True)
    report.write_text(
        render(data, title="Kerna — demo (synthetic data)"), encoding="utf-8")

    print()
    print(f"  requests recorded        {data['requests']}")
    print(f"  policy decisions         {data['gov_checks']}")
    print(f"  denied but not enforced  {data['gov_denied_observed']}")
    print(f"  cost + policy joined     {data['turns_cost_and_policy']}")
    print(f"  tool calls gated         {data['gate_seen']}")
    print(f"  blocked by policy        {data['denials']}")
    print(f"  turns in all three logs  {data['turns_in_all_three']}"
          "   (needs a local model; this demo has none)")
    if note:
        print(f"\n  note: {note}")
    print(f"\n  report: {report}")

    if not args.no_open:
        webbrowser.open(report.resolve().as_uri())

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
