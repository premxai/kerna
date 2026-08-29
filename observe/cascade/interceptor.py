"""The interceptor — an OpenAI-compatible endpoint that is safe to put in the path.

Installed with one environment variable and removed by unsetting it:

    OPENAI_BASE_URL=http://127.0.0.1:8127/v1

That reversibility is a selling point and should be said out loud in a first meeting
(§10 of the playbook). It is also the honest answer to "what if your thing breaks" —
the customer's escape is one variable, not a support ticket.

## The one rule

**Nothing that happens inside this process may make a request worse than it would have
been without us.** Every branch below either returns a local answer that passed the
customer's own gate, or forwards upstream exactly as if we did not exist. There is no
third outcome, including when our own code is broken: `handle_completion` catches its
own failures and escalates, because a stack trace in our dispatcher is not the
customer's problem.

## We never hold the customer's key

The client's `Authorization` header is passed through to the upstream provider
untouched, and never stored, logged, or copied into our own state. Decision 028 says
the customer keeps their provider relationship; this is that decision expressed as
plumbing. It also means an install requires no credential from us and leaks none if the
sidecar is compromised — there is nothing here to take.

## What is deliberately missing

Local routing is **off** (see `dispatcher.LOCAL_ROUTING_ENABLED`) until the C0a fairness
test has run. Everything in this module works today; it simply forwards. That is the
intended state — the plumbing is exercised in production shape long before the claim it
enables is earned.
"""

from __future__ import annotations

import json
import time
from pathlib import Path
from dataclasses import dataclass
from typing import Any, Callable

from . import correlate
from .datadir import default_log, ensure_parent
from .circuit import CircuitBreaker
from .classify import Turn, classify
from .dialect import to_openai_chat
from .dispatcher import Dispatcher, Route
from .enforce import ToolCallGate
from .explore import (
    DEFAULT_IDLE_AFTER_S,
    DEFAULT_QUEUE,
    DEFAULT_TTL_S,
    ContextIneligible,
    ExploreItem,
    Explorer,
)
from .oracle import StreamActionAccumulator
from .synth import local_stream
from .tool_catalog import textual_catalog
from .tool_grammar import build_tool_call_grammar, offered_tools
from .tool_policy import TOOL_POLICIES, filter_tools
from .recorder import TrafficRecorder

# Deliberately not 8080: that is llama-server's default in every doc in this repo, and a
# sidecar that silently collides with the model server would be diagnosed as "the model
# is broken" for an afternoon.
DEFAULT_PORT = 8127

# Headers we must not forward. Hop-by-hop headers and any length/encoding that describes
# the *original* body — we re-serialise, so a stale content-length truncates the upstream
# request and the failure looks like a provider outage.
_STRIP_HEADERS = {
    "host", "content-length", "connection", "keep-alive", "transfer-encoding",
    "upgrade", "proxy-authorization", "proxy-connection", "te", "trailer",
    "accept-encoding",
}


@dataclass
class CascadeResult:
    payload: dict[str, Any]
    status: int
    path: str      # "cloud" | "local" | "local->cloud" | "explore->cloud"
    reason: str
    elapsed_ms: float = 0.0

    @property
    def served_locally(self) -> bool:
        return self.path == "local"


def forwardable_headers(headers: dict[str, str]) -> dict[str, str]:
    """The client's headers, minus what must not be relayed. Authorization survives."""
    return {k: v for k, v in headers.items() if k.lower() not in _STRIP_HEADERS}


def handle_completion(
    payload: dict[str, Any],
    headers: dict[str, str],
    *,
    dispatcher: Dispatcher,
    forward_cloud: Callable[[dict[str, Any], dict[str, str]], tuple[dict[str, Any], int]],
    attempt_local: Callable[[dict[str, Any]], dict[str, Any] | None] | None = None,
    task_class: str | None = None,
    turn: Turn | None = None,
    explorer: Explorer | None = None,
    turn_id: str | None = None,
    clock: Callable[[], float] = time.monotonic,
) -> CascadeResult:
    """Serve one chat-completion request. Never raises.

    `attempt_local` returns the completed payload on convergence, or **None** when the
    local path did not converge — a normal, expected outcome that escalates. It raises
    only for infrastructure faults, which additionally trip the breaker. Keeping those
    two failure kinds distinct is the whole reason the breaker does not open on hard
    task classes (see `circuit.record_failure`).
    """
    started = clock()

    def _finish(result: CascadeResult) -> CascadeResult:
        result.elapsed_ms = (clock() - started) * 1000.0
        return result

    verdict = dispatcher.decide(payload, task_class=task_class, turn=turn)

    if verdict.route is Route.LOCAL and attempt_local is not None:
        try:
            local = attempt_local(payload)
        except Exception as exc:  # noqa: BLE001
            # Infrastructure: the model server died, the sandbox is gone, a driver
            # update landed. Trip the breaker and escalate this request.
            dispatcher.breaker.record_failure()
            body, status = forward_cloud(payload, forwardable_headers(headers))
            return _finish(CascadeResult(
                body, status, "local->cloud",
                f"local_infrastructure_failure:{type(exc).__name__}",
            ))

        if local is not None:
            dispatcher.breaker.record_success()
            return _finish(CascadeResult(local, 200, "local", verdict.reason))

        # Did not converge. Not a fault; the cascade is doing exactly its job.
        dispatcher.breaker.record_escalation()
        body, status = forward_cloud(payload, forwardable_headers(headers))
        return _finish(CascadeResult(body, status, "local->cloud", "did_not_converge"))

    path = "explore->cloud" if verdict.route is Route.EXPLORE else "cloud"
    body, status = forward_cloud(payload, forwardable_headers(headers))

    # The customer's answer is complete. Only now is exploration allowed to exist, and
    # only as a queue put that cannot block or raise -- everything else it might do
    # happens on another thread, later, when nobody is waiting.
    if verdict.route is Route.EXPLORE and explorer is not None:
        try:
            explorer.submit(ExploreItem(
                payload=payload,
                cloud_answer=body if isinstance(body, dict) else None,
                task_class=task_class or (turn.task_class if turn else "unknown"),
                queued_at=clock(),
                edits_since_check=turn.edits_since_check if turn else 0,
                turn=turn_id,
            ))
        except Exception:  # noqa: BLE001
            # Submitting is one non-blocking put and should be incapable of failing.
            # It is wrapped anyway, because the alternative to this `except` is a
            # customer's request failing for the sake of a background measurement --
            # and that trade is never worth making, however unlikely.
            pass

    return _finish(CascadeResult(body, status, path, verdict.reason))


# ------------------------------------------------------------------ upstream


def stream_passthrough(
    upstream: str,
    endpoint: str,
    payload: dict[str, Any],
    headers: dict[str, str],
    write: Callable[[bytes], None],
    *,
    on_response: Callable[[int, dict[str, str]], None] | None = None,
    inspect: Callable[[bytes], None] | None = None,
    timeout_s: float = 600.0,
    gate: ToolCallGate | None = None,
) -> tuple[int, dict[str, Any]]:
    """Relay a streaming response byte-for-byte and report what went past.

    Streaming is not an optimisation here, it is a correctness requirement: coding agents
    stream, and a proxy that buffers a stream to inspect it turns a responsive tool into
    one that appears frozen until the whole answer lands. Chunks are written straight
    through as they arrive.

    Nothing is parsed out of the body. The only things counted are bytes and events, plus
    `usage` if the provider happens to send it in a terminal event — enough to know what
    the traffic looked like, without holding the customer's source in memory or making
    the relay depend on any provider's event schema.

    ## The one exception, and it is opt-in

    `gate` is the native-tool enforcement point (Decision 041). With no gate this function
    is byte-for-byte, exactly as before. With one, **text still streams through untouched
    and only a `tool_use` block is ever held** -- from its opening frame to its closing
    one, a few hundred bytes at the end of a turn.

    A gate that raises cannot break the relay: it degrades itself to passthrough, flushes
    what it was holding, and records that it did. That is the opposite default from the
    policy decision inside it, which fails closed. **Plumbing fails open; decisions fail
    closed**, and conflating the two is the bug this is written to avoid.
    """
    import httpx

    meta: dict[str, Any] = {"bytes": 0, "events": 0}
    base = upstream.rstrip("/")
    # A bounded tail, not the whole body. Providers report token usage in a terminal
    # event, and without it a streamed request looks free next to a blocking one that
    # reports its cost — which would make any comparison between the two meaningless.
    # Capped so a long stream cannot grow this in memory.
    tail = bytearray()
    TAIL_CAP = 8192

    # `identity` is mandatory, not a preference.
    #
    # httpx sends `accept-encoding: gzip, deflate` by default, and `iter_raw()` hands
    # back the body STILL COMPRESSED. Relaying those bytes under a `text/event-stream`
    # content type without a matching `Content-Encoding` gives the client gzip it has
    # been told is plain text: it cannot parse the stream, gives up, and reissues the
    # turn as a blocking request.
    #
    # That is the entire "every turn is sent twice" mystery. It was read as client
    # behaviour and recorded as a finding about agent traffic, and it was our own relay
    # corrupting every stream it touched. Asking upstream not to compress keeps
    # `iter_raw()` honest and the relay genuinely byte-for-byte.
    upstream_headers = {**headers, "accept-encoding": "identity"}

    with httpx.Client(timeout=timeout_s) as client:
        with client.stream(
            "POST", f"{base}/{endpoint}", json=payload, headers=upstream_headers
        ) as response:
            status = response.status_code
            if on_response is not None:
                on_response(status, dict(response.headers))
            for chunk in response.iter_raw():
                if not chunk:
                    continue
                meta["bytes"] += len(chunk)
                meta["events"] += chunk.count(b"\ndata:") + chunk.count(b"data:")
                if inspect is not None:
                    inspect(chunk)
                if gate is None:
                    write(chunk)
                else:
                    for out in gate.feed(chunk):
                        write(out)
                # `usage` is read from what UPSTREAM sent, not from what we forwarded. A
                # denial rewrites the stream, and the customer was still billed for the
                # tokens that produced it ? a report reading its own output would
                # understate spend at exactly the moment enforcement did something.
                tail.extend(chunk)
                if len(tail) > TAIL_CAP:
                    del tail[:-TAIL_CAP]

            if gate is not None:
                for out in gate.finish():
                    write(out)

    usage = _usage_from_tail(bytes(tail))
    if usage:
        meta["usage"] = usage
    if gate is not None:
        meta["gate"] = gate.stats.as_dict()
    return status, meta


def _usage_from_tail(tail: bytes) -> dict[str, Any] | None:
    """Pull the usage record out of the last events of a stream, if one is there.

    Parsed defensively and last-wins: providers differ on which terminal event carries
    the totals, the tail may begin mid-event, and none of that is worth failing a relay
    over. Returning None simply means the cost of that request is unknown, which is the
    honest outcome rather than a fabricated zero.
    """
    found: dict[str, Any] | None = None
    for line in tail.split(b"\n"):
        line = line.strip()
        if not line.startswith(b"data:"):
            continue
        body = line[5:].strip()
        if not body or body == b"[DONE]":
            continue
        try:
            event = json.loads(body)
        except ValueError:
            continue    # truncated first event in the tail window
        usage = event.get("usage") or (event.get("message") or {}).get("usage")
        if isinstance(usage, dict) and usage:
            merged = dict(found or {})
            merged.update(usage)
            found = merged
    return found


def make_cloud_forwarder(
    upstream: str, *, timeout_s: float = 600.0, endpoint: str = "chat/completions"
) -> Callable[[dict[str, Any], dict[str, str]], tuple[dict[str, Any], int]]:
    """Forward a request to the customer's own provider, verbatim.

    The generous timeout is not carelessness: this is the *fallback* path, and a
    fallback that gives up sooner than the provider does would manufacture failures the
    customer would not have had without us.
    """
    import httpx

    base = upstream.rstrip("/")
    client = httpx.Client(timeout=timeout_s)

    def forward(payload: dict[str, Any], headers: dict[str, str]) -> tuple[dict[str, Any], int]:
        response = client.post(
            f"{base}/{endpoint}", json=payload, headers=headers
        )
        try:
            return response.json(), response.status_code
        except ValueError:
            # A non-JSON body from upstream is still upstream's answer; relay it rather
            # than inventing an error of our own.
            return {"error": {"message": response.text[:500]}}, response.status_code

    return forward


# ------------------------------------------------------------------ HTTP server


def build_handler(
    *,
    dispatcher: Dispatcher,
    forward_cloud: Callable[[dict[str, Any], dict[str, str]], tuple[dict[str, Any], int]],
    attempt_local: Callable[[dict[str, Any]], dict[str, Any] | None] | None,
    explorer: Explorer | None = None,
    upstream: str | None = None,
    recorder: TrafficRecorder | None = None,
    gate_policy: Any | None = None,
    gate_mode: str = "observe",
    record_gate: Callable[[dict[str, Any]], None] | None = None,
    quiet: bool = False,
):
    """Build the BaseHTTPRequestHandler class, closed over its dependencies."""
    from http.server import BaseHTTPRequestHandler

    from .enforce import Mode as GateMode
    from .enforce import Policy as GatePolicy

    # A gate is stateful for the length of one response, so it is built per request
    # rather than shared. `None` policy means the permissive default: rung 1 records
    # what a policy would have stopped and stops nothing, which is the only honest
    # install default (040).
    gate_policy = gate_policy if gate_policy is not None else GatePolicy()
    gate_enabled = gate_mode != "off"

    def new_gate(turn_id: str | None):
        if not gate_enabled:
            return None
        return ToolCallGate(policy=gate_policy, mode=GateMode(gate_mode),
                            turn=turn_id,
                            record=record_gate or (lambda row: None))

    base = upstream.rstrip("/") if upstream else None
    blocking_forwarders = {"chat/completions": forward_cloud}
    if upstream:
        blocking_forwarders["messages"] = make_cloud_forwarder(
            upstream, endpoint="messages"
        )

    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, fmt: str, *args) -> None:  # noqa: A003
            if not quiet:
                super().log_message(fmt, *args)

        def _send(self, body: dict[str, Any], status: int) -> None:
            encoded = json.dumps(body).encode("utf-8")
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(encoded)))
            self.end_headers()
            self.wfile.write(encoded)

        def _endpoint(self) -> str | None:
            clean = self.path.split("?", 1)[0].rstrip("/")
            if clean == "/v1/messages":
                return "messages"
            if clean == "/v1/chat/completions":
                return "chat/completions"
            return None

        def _upstream_url(self) -> str:
            if base is None:
                raise RuntimeError("no upstream configured")
            if self.path == "/v1":
                return base
            if self.path.startswith("/v1/"):
                return f"{base}{self.path[3:]}"
            return f"{base}{self.path}"

        def _relay_other(self, method: str, body: bytes | None = None) -> None:
            if base is None:
                self._send({"error": {"message": "not found"}}, 404)
                return

            import httpx

            headers = forwardable_headers({k: v for k, v in self.headers.items()})
            try:
                with httpx.Client(timeout=600.0) as client:
                    response = client.request(
                        method,
                        self._upstream_url(),
                        content=body,
                        headers=headers,
                    )
                data = response.content
                self.send_response(response.status_code)
                self.send_header(
                    "Content-Type",
                    response.headers.get("content-type", "application/json"),
                )
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                if data:
                    self.wfile.write(data)
            except Exception as exc:  # noqa: BLE001
                self._send(
                    {"error": {"message": f"relay failed: {type(exc).__name__}"}},
                    502,
                )

        def do_GET(self) -> None:  # noqa: N802
            if self.path.rstrip("/") in ("/health", "/v1/health"):
                self._send({
                    "status": "ok",
                    "local_routing": dispatcher.local_routing_enabled,
                    "breaker": dispatcher.breaker.describe(),
                }, 200)
                return

            # Claude Code performs model/capability probes before the first message.
            # A transparent sidecar must relay them rather than answering 404 itself.
            self._relay_other("GET")

        def do_POST(self) -> None:  # noqa: N802
            length = int(self.headers.get("Content-Length") or 0)
            raw = self.rfile.read(length) if length else b"{}"
            headers = {k: v for k, v in self.headers.items()}

            endpoint = self._endpoint()

            # Examples include /v1/messages/count_tokens. These are not routing
            # opportunities; they are simply part of Claude Code's provider protocol.
            if endpoint is None:
                self._relay_other("POST", raw)
                return

            try:
                payload = json.loads(raw or b"{}")
            except ValueError:
                self._send(
                    {"error": {"message": "request body was not valid JSON"}},
                    400,
                )
                return

            # One id for this turn, before any component writes a row about it.
            # Inherited when a caller supplied one -- that is what lets Kerna's audit
            # trail and our logs describe the same timeline (Decision 041).
            this_turn = correlate.turn_id(headers)

            if explorer is not None:
                explorer.note_request()

            try:
                turn = classify(payload)
            except Exception:  # noqa: BLE001
                turn = None

            # Claude Code streams its agentic turns. The customer still receives the
            # upstream stream byte-for-byte; the only thing retained for EXPLORE is the
            # cloud tool action required for the comparison.
            if payload.get("stream") and upstream is not None:
                started = time.monotonic()
                verdict = dispatcher.decide(payload, turn=turn)
                path = (
                    "explore->cloud"
                    if verdict.route is Route.EXPLORE
                    else "cloud"
                )

                accumulator = StreamActionAccumulator()
                pending = bytearray()
                headers_sent = False
                gate = new_gate(this_turn)

                def on_response(
                    status: int, response_headers: dict[str, str]
                ) -> None:
                    nonlocal headers_sent
                    self.send_response(status)
                    self.send_header(
                        "Content-Type",
                        response_headers.get(
                            "content-type", "text/event-stream"
                        ),
                    )
                    self.send_header(
                        "Cache-Control",
                        response_headers.get("cache-control", "no-cache"),
                    )
                    self.send_header("Connection", "close")
                    self.end_headers()
                    self.close_connection = True
                    headers_sent = True

                def inspect(chunk: bytes) -> None:
                    # Feed complete SSE lines to the action accumulator. TCP chunks are
                    # not guaranteed to align with event boundaries.
                    pending.extend(chunk)
                    while True:
                        pos = pending.find(b"\n")
                        if pos < 0:
                            break
                        line = bytes(pending[:pos + 1])
                        del pending[:pos + 1]
                        accumulator.feed(line)

                def write(chunk: bytes) -> None:
                    self.wfile.write(chunk)
                    self.wfile.flush()

                # Local serving, on the path that carries every agentic turn. Until
                # stream synthesis existed this branch could only forward, which is why
                # `local_can_stream` was False: 6 of 6 agentic turns stream, so a serving
                # path that cannot stream serves nothing at all.
                #
                # Fail open (030): a local attempt that does not converge, or throws,
                # falls through to the cloud exactly as if we were not here.
                if (verdict.route is Route.LOCAL
                        and attempt_local is not None
                        and dispatcher.local_can_stream):
                    try:
                        local = attempt_local(payload)
                    except Exception:  # noqa: BLE001
                        dispatcher.breaker.record_failure()
                        local = None

                    if local is not None:
                        dispatcher.breaker.record_success()
                        on_response(200, {})
                        # Through the same gate the cloud path uses. This is the route
                        # where the model is the one the customer has NOT agreed to
                        # trust, so ungoverned local serving would be the wrong way
                        # round.
                        for chunk in local_stream(local, anthropic=(endpoint == "messages"),
                                                  gate=gate):
                            write(chunk)
                        return
                    dispatcher.breaker.record_escalation()

                try:
                    status, response_meta = stream_passthrough(
                        upstream,
                        endpoint,
                        payload,
                        forwardable_headers(headers),
                        write,
                        on_response=on_response,
                        inspect=inspect,
                        gate=gate,
                    )
                    if pending:
                        accumulator.feed(bytes(pending))
                except Exception as exc:  # noqa: BLE001
                    if not headers_sent:
                        self._send({
                            "error": {
                                "message":
                                    f"upstream stream failed: {type(exc).__name__}"
                            }
                        }, 502)
                    self.close_connection = True
                    return

                elapsed = (time.monotonic() - started) * 1000.0

                if recorder is not None:
                    recorder.record(
                        endpoint,
                        payload,
                        route=path,
                        reason=verdict.reason,
                        status=status,
                        elapsed_ms=elapsed,
                        response_meta=response_meta,
                        headers=headers,
                        turn=this_turn,
                    )

                if verdict.route is Route.EXPLORE and explorer is not None:
                    try:
                        action = accumulator.action()

                        # Existing Explorer expects a completed response object.
                        # Store only the cloud tool call, not the cloud prose.
                        cloud_answer = None
                        if action is not None and not action.truncated:
                            cloud_answer = {
                                "content": [{
                                    "type": "tool_use",
                                    "name": action.tool,
                                    "input": action.args,
                                }]
                            }

                        explorer.submit(ExploreItem(
                            payload=payload,
                            cloud_answer=cloud_answer,
                            task_class=(
                                turn.task_class if turn else "unknown"
                            ),
                            queued_at=time.monotonic(),
                            edits_since_check=(
                                turn.edits_since_check if turn else 0
                            ),
                            turn=this_turn,
                        ))
                    except Exception:  # noqa: BLE001
                        pass

                if not quiet:
                    print(
                        f"  {path:<14}{verdict.reason:<52}"
                        f"{elapsed:7.0f} ms"
                    )
                return

            cloud_forward = blocking_forwarders.get(endpoint, forward_cloud)

            try:
                result = handle_completion(
                    payload, headers,
                    dispatcher=dispatcher,
                    forward_cloud=cloud_forward,
                    attempt_local=attempt_local,
                    turn=turn,
                    explorer=explorer,
                    turn_id=this_turn,
                )
            except Exception as exc:  # noqa: BLE001
                # handle_completion is written not to raise; this is the belt on top of
                # the braces. If it ever fires, the customer still gets an answer.
                try:
                    body, status = forward_cloud(payload, forwardable_headers(headers))
                    self._send(body, status)
                except Exception:  # noqa: BLE001
                    self._send({"error": {
                        "message": f"cascade and upstream both failed: {type(exc).__name__}"
                    }}, 502)
                return

            if recorder is not None:
                response_meta = None
                if isinstance(result.payload, dict):
                    usage = result.payload.get("usage")
                    if isinstance(usage, dict):
                        response_meta = {"usage": usage}

                recorder.record(
                    endpoint,
                    payload,
                    route=result.path,
                    reason=result.reason,
                    status=result.status,
                    elapsed_ms=result.elapsed_ms,
                    response_meta=response_meta,
                    headers=headers,
                    turn=this_turn,
                )

            if not quiet:
                print(f"  {result.path:<14}{result.reason:<52}{result.elapsed_ms:7.0f} ms")
            self._send(result.payload, result.status)

    return Handler


def serve(
    *,
    upstream: str,
    port: int = DEFAULT_PORT,
    dispatcher: Dispatcher | None = None,
    attempt_local: Callable[[dict[str, Any]], dict[str, Any] | None] | None = None,
    explorer: Explorer | None = None,
    recorder: TrafficRecorder | None = None,
    # Echoed at startup, not used for opening. A path that resolves somewhere other
    # than the operator expects yields a run that looks healthy and writes its
    # evidence where nobody will look for it.
    traffic_log: Path | None = None,
    explore_log: Path | None = None,
    gate_policy: Any | None = None,
    gate_mode: str = "observe",
    serve_local: bool = False,
    ledger_path: Path | None = None,
    machine_tier: str | None = None,
    local_model: str | None = None,
    quiet: bool = False,
) -> None:
    from http.server import ThreadingHTTPServer

    from .dispatcher import gate_status

    # The ledger decides *what* may be served locally; `serve_local` decides whether
    # anything may be at all. Both are required, and they are separate on purpose: the
    # flag is an operator's per-install choice, the promotions are earned evidence, and
    # neither one substitutes for the other.
    promoted: frozenset[str] = frozenset()
    ledger = None
    if ledger_path is not None:
        from .ledger import Ledger
        from .promote import promoted_for

        from .promote import feed, read_rows

        ledger = Ledger.load(Path(ledger_path))

        # Catch up on evidence written by earlier runs before deciding anything. Feeding
        # is idempotent -- rows carry a turn id and the ledger remembers which it has
        # seen -- so re-reading a log a restart already consumed cannot inflate the
        # trials behind a promotion.
        if machine_tier and explore_log is not None:
            caught = feed(ledger, read_rows(Path(explore_log)),
                          machine_tier=machine_tier)
            if caught["recorded"] and not quiet:
                print(f"  ledger: {caught['recorded']} new observations "
                      f"({caught['scored']} scoreable, {caught['duplicate']} already "
                      f"counted)")
            if caught["recorded"]:
                ledger.save()

        if machine_tier and local_model:
            promoted = promoted_for(
                ledger, machine_tier=machine_tier, model=local_model)

        # Every new observation goes to the ledger as it happens, so a long-running
        # pilot promotes without needing a restart. The explorer's own log write stays
        # first: evidence on disk matters more than evidence in memory if this crashes.
        if explorer is not None and machine_tier:
            _wrap_explorer_record(explorer, ledger, machine_tier)

    dispatcher = dispatcher or Dispatcher(
        breaker=CircuitBreaker(),
        idle=explorer is not None,
        local_routing_enabled=serve_local,
        promoted_classes=promoted,
    )

    if not quiet:
        if serve_local and promoted:
            print(f"  serving locally: {', '.join(sorted(promoted))}  "
                  f"(earned on {machine_tier} / {local_model})")
        elif serve_local:
            print("  --serve-local is on and NOTHING has earned promotion yet, so every "
                  "request still goes to the cloud. That is the ledger working.")
    handler = build_handler(
        dispatcher=dispatcher,
        forward_cloud=make_cloud_forwarder(upstream),
        attempt_local=attempt_local,
        explorer=explorer,
        upstream=upstream,
        recorder=recorder,
        gate_policy=gate_policy,
        gate_mode=gate_mode,
        record_gate=explorer.record if explorer is not None else None,
        quiet=quiet,
    )
    class ExclusiveServer(ThreadingHTTPServer):
        """A sidecar must never share its port, and on Windows the default lets it.

        `ThreadingHTTPServer.allow_reuse_address` is 1. On Unix that only permits reuse
        of a socket in TIME_WAIT, which is what you want when restarting a server. On
        **Windows, SO_REUSEADDR lets a second socket bind an address another process is
        actively listening on** — both succeed, and the OS hands each new connection to
        one of them with no rule anyone can rely on.

        Measured here, expensively. A sidecar left running from an earlier test still
        held 8127; a second one started, reported itself healthy, wrote its own traffic
        log, and served none of the requests. Every test hit the older process and got
        that process's upstream, so a request sent with no API key came back 200 with a
        plausible answer. Two "running" sidecars, evidence split across two files with
        different upstreams, and nothing in either log saying so.

        For a product whose only output is evidence, silently splitting that evidence is
        the worst failure available. Refusing the second bind turns it into an error
        message.
        """

        allow_reuse_address = False

    try:
        server = ExclusiveServer(("127.0.0.1", port), handler)
    except OSError as exc:
        # Found the hard way: a stale sidecar already held this port, so the new process
        # wrote its traffic-log header, failed here, and said nothing anyone saw. The
        # log looked like a healthy run that had recorded no traffic yet, and every
        # request went to the *old* process -- which answered, with its own upstream.
        #
        # A silent bind failure on a monitoring tool is worse than a crash: the operator
        # believes they are measuring their traffic and they are measuring something
        # else. Say what happened, say the likely cause, and exit non-zero.
        raise SystemExit(
            f"cannot listen on 127.0.0.1:{port}: {exc}\n"
            f"  Another process is probably already using it -- an earlier sidecar that "
            f"is still running, or another service.\n"
            f"  Stop it, or choose a different port with --port."
        ) from exc

    print(f"cascade sidecar on http://127.0.0.1:{port}  ->  {upstream}")
    print(f"  {gate_status(serve_local)}")
    if explorer is not None:
        explorer.start()
        print(f"  EXPLORE mode: attempts run on the idle budget and are never served.")
        print(f"    idle after {explorer.idle_after_s:.0f}s · queue {explorer.queue_size}"
              f" · checks {'on' if explorer.repo_root else 'OFF (no --repo)'}")
    # Printed rather than documented. A path that resolves somewhere other than the
    # operator expects produces a run that looks healthy and writes its evidence where
    # nobody will look for it.
    if traffic_log is not None:
        print(f"  traffic:   {traffic_log}")
    if explore_log is not None:
        print(f"  explore:   {explore_log}")
    print(f"  governance: {gate_mode}"
          + ("" if gate_policy is not None else "  (permissive policy: records, denies nothing)"))
    print(f"  install:   OPENAI_BASE_URL=http://127.0.0.1:{port}/v1")
    print(f"  uninstall: unset OPENAI_BASE_URL")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
        if explorer is not None:
            explorer.close()
            print()
            print(f"exploration: {json.dumps(explorer.stats.as_dict())}")


def main(argv: list[str] | None = None) -> int:
    import argparse

    parser = argparse.ArgumentParser(
        prog="cascade",
        description="OpenAI-compatible sidecar. Forwards everything; fails open always.",
    )
    parser.add_argument("--upstream", default="https://api.openai.com/v1",
                        help="the customer's own provider base URL")
    parser.add_argument("--port", type=int, default=DEFAULT_PORT)
    parser.add_argument("--quiet", action="store_true")
    parser.add_argument("--explore", metavar="LOCAL_URL", default=None,
                        help="attempt eligible turns against this local server, on the "
                             "idle budget, and never serve the result "
                             "(e.g. http://127.0.0.1:8080)")
    parser.add_argument("--explore-log", type=Path, default=default_log("explore.jsonl"),
                        help="where EXPLORE rows are written (default: per-user data dir)")
    parser.add_argument(
        "--tool-policy",
        choices=TOOL_POLICIES,
        default="full",
        help="tool menu offered only to the local EXPLORE model",
    )
    parser.add_argument(
        "--local-tool-choice",
        choices=("auto", "required"),
        default="auto",
        help="local EXPLORE tool-choice cohort; never changes the cloud request",
    )
    parser.add_argument(
        "--local-max-tokens",
        type=int,
        default=None,
        help="cap output tokens only for the local EXPLORE request",
    )
    parser.add_argument(
        "--local-tool-prompt",
        choices=("none", "strict-v1"),
        default="none",
        help="local-only tool-use prompting cohort",
    )
    parser.add_argument(
        "--local-decoding",
        choices=LOCAL_DECODINGS,
        default="none",
        help="constrain the local shadow response. grammar-v1 makes prose "
             "unrepresentable, so an action always exists to compare -- "
             "tool_choice=required turned out to be a hint, not a constraint",
    )
    parser.add_argument(
        "--local-tool-transport",
        choices=LOCAL_TOOL_TRANSPORTS,
        default="native",
        help="how the local shadow is shown its tools. textual-v1 serialises the "
             "surviving menu into the system prompt and drops the tools field, "
             "which is what makes a custom grammar possible at all: llama.cpp "
             "returns HTTP 400 for grammar plus tools together",
    )
    parser.add_argument(
        "--local-dialect", choices=("auto", "openai"), default="auto",
        help="which API the local server speaks. `auto` mirrors the client's own "
             "dialect, which is right for llama.cpp because it serves both. Ollama "
             "serves only the OpenAI one, so a Claude Code turn shadowed against it "
             "404s and every row reads did_not_converge -- pass `openai` there",
    )
    parser.add_argument(
        "--serve-local", action="store_true",
        help="allow a PROMOTED task class to be answered by the local model. Off by "
             "default: on a matched fairness test local slipped 8.1%% of wrong answers "
             "past the correctness gate against the cloud's 3.8%%, and the interval on "
             "that difference contained both zero and the kill threshold. A pilot may "
             "flip this per install; a class still has to earn promotion first",
    )
    parser.add_argument(
        "--ledger", type=Path, default=default_log("ledger.json"),
        help="the autonomy ledger: which classes have earned local execution on this "
             "machine, with this model (default: per-user data dir)",
    )
    parser.add_argument(
        "--gate-mode",
        choices=("off", "observe", "shadow", "enforce"),
        default="observe",
        help="governance at the model seam (040's ladder). observe records what a "
             "policy would have stopped and stops nothing, which is the only honest "
             "install default; shadow decides and changes nothing; enforce denies",
    )
    parser.add_argument(
        "--policy-file",
        type=Path,
        default=None,
        help="JSON policy in the shared vocabulary of docs/POLICY.md. Without one the "
             "policy is permissive, so observe mode records tool calls and denies none",
    )
    parser.add_argument(
        "--local-model",
        default=None,
        help="label for the local model, if the server cannot be asked. The server's "
             "own answer wins: a typed label is a claim that drifts the first time "
             "someone swaps a GGUF and forgets the flag",
    )
    parser.add_argument(
        "--record-action-args",
        action="store_true",
        help="also record each proposed action's target (usually a file path in the "
             "customer's repository). Tool names are always recorded; this adds the "
             "arguments, which is what makes a disagreement legible",
    )
    parser.add_argument("--traffic-log", type=Path, default=default_log("traffic.jsonl"),
                        help="record request structure and provider usage for savings "
                             "analysis (default: per-user data dir)")
    parser.add_argument("--repo", type=Path, default=None,
                        help="repository to run the sandbox check in; without it, rows "
                             "carry agreement and no check verdict")
    parser.add_argument("--test-command", default=None,
                        help="override the discovered test command")
    parser.add_argument("--idle-after", type=float, default=DEFAULT_IDLE_AFTER_S,
                        help="seconds of quiet before an attempt may run")
    parser.add_argument("--queue", type=int, default=DEFAULT_QUEUE,
                        help="how many turns may wait to be explored")
    parser.add_argument("--ttl", type=float, default=DEFAULT_TTL_S,
                        help="seconds a queued turn stays worth exploring. The default "
                             "suits continuous use; a long working session followed by a "
                             "single idle window needs it raised, or the earliest turns "
                             "expire before the queue ever drains.")
    parser.add_argument("--allow-host-execution", action="store_true",
                        help="run the suite outside a container. The edit under test was "
                             "written by a model and reviewed by nobody.")
    args = parser.parse_args(argv)

    # Measured, not defensive: llama.cpp returns HTTP 400 for a request carrying both a
    # custom `grammar` and a `tools` field. Left to run, this combination fails on every
    # turn that has tools -- which the interceptor classifies as an infrastructure fault,
    # so the breaker opens and the session reports a broken local server rather than a
    # bad flag. A whole cohort has already been lost to a configuration that looked like
    # it applied and did not; refusing at startup costs one line and one second.
    if args.local_decoding.startswith("grammar") and args.local_tool_transport == "native":
        parser.error(
            f"--local-decoding {args.local_decoding} requires a textual tool transport: "
            "llama.cpp rejects a custom grammar sent alongside the native tools field "
            "(HTTP 400), so the menu has to travel in the prompt"
        )

    # The grammar and the catalog have to agree about abstention. A grammar admitting
    # __no_tool__ while the prompt never mentions it is a trap the model cannot see; a
    # prompt offering it while the grammar forbids it makes every token illegal.
    abstain_grammar = args.local_decoding == "grammar-v2"
    abstain_catalog = args.local_tool_transport == "textual-v2"
    if abstain_grammar != abstain_catalog and args.local_decoding != "none":
        parser.error(
            "grammar-v2 and textual-v2 go together: one admits the __no_tool__ option "
            "and the other explains it. Pair grammar-v1 with textual-v1, or grammar-v2 "
            "with textual-v2 -- never across versions"
        )

    gate_policy = None
    if args.policy_file:
        from .enforce import load_policy

        # Raises on a bad path or bad JSON rather than falling back to permissive.
        # Governance is fail-closed on decisions, and silently allowing everything
        # because a filename was mistyped is the one failure it must never have.
        gate_policy = load_policy(args.policy_file)

    recorder = TrafficRecorder(path=args.traffic_log)

    explorer = None
    if args.explore:
        explorer = _build_explorer(args)

    # Which machine and which model this evidence belongs to. A class earned on a 24 GB
    # workstation says nothing about an 8 GB laptop, and pooling them would promote work
    # onto hardware that has never run it -- so an unknown tier or model promotes nothing
    # rather than falling back to a permissive default.
    tier, model = _cohort_identity(args)

    serve(
        upstream=args.upstream,
        port=args.port,
        quiet=args.quiet,
        explorer=explorer,
        recorder=recorder,
        gate_policy=gate_policy,
        gate_mode=args.gate_mode,
        traffic_log=args.traffic_log,
        explore_log=args.explore_log if args.explore else None,
        serve_local=args.serve_local,
        ledger_path=args.ledger,
        machine_tier=tier,
        local_model=model,
    )
    return 0


def _wrap_explorer_record(explorer, ledger, machine_tier: str) -> None:
    """Also record each explore row into the ledger, without ever failing the run.

    The explorer's contract is that its worker cannot raise into anything. Feeding the
    ledger must not be the thing that breaks it, so a failure here is swallowed: losing
    one observation is a slightly slower promotion, while raising would take out the
    background loop that produces all of them.
    """
    from .promote import feed

    original = explorer.record
    counter = {"n": 0}

    def also_ledger(row: dict[str, Any]) -> None:
        original(row)
        try:
            feed(ledger, [row], machine_tier=machine_tier)
            counter["n"] += 1
            # Persist periodically rather than per row: a torn write is guarded by the
            # atomic save, but a save on every observation would fsync in the path of
            # the idle worker for no benefit.
            if counter["n"] % 10 == 0:
                ledger.save()
        except Exception:  # noqa: BLE001
            pass

    explorer.record = also_ledger


def _cohort_identity(args) -> tuple[str | None, str | None]:
    """(machine_tier, local_model) for scoping the ledger, or (None, None).

    Both are probed rather than assumed. Failing to identify either means no class is
    promoted, which is the safe direction: everything goes to the cloud, exactly as on a
    fresh install.
    """
    if not getattr(args, "explore", None):
        return None, None
    try:
        from ..registry.device import profile

        tier = profile().tier.value
    except Exception:  # noqa: BLE001
        tier = None
    try:
        from .identity import resolve_model

        model, _ = resolve_model(args.explore, None)
    except Exception:  # noqa: BLE001
        model = None
    return tier, (model if model and model != "unknown" else None)


def _build_explorer(args) -> Explorer:
    """Assemble the exploration path from CLI arguments.

    Kept out of `main` because two of these choices are worth reading rather than
    skimming: without `--repo` there is no check and rows carry only agreement, and
    `--allow-host-execution` runs a model's unreviewed edit outside a container.
    """
    from .check import discover_test_command

    test_command = None
    if args.repo:
        test_command = (args.test_command.split() if args.test_command
                        else discover_test_command(args.repo))
        if test_command is None:
            print(f"!! no test suite found in {args.repo} — the behaviour arm will be "
                  f"unmeasured. Pass --test-command to name it.")

    log = ensure_parent(args.explore_log).open("a", encoding="utf-8")

    def record(row: dict[str, Any]) -> None:
        log.write(json.dumps(row, ensure_ascii=False) + "\n")
        log.flush()          # a crashed session must not lose the evidence it gathered

    from .identity import identity_row, resolve_model

    model, source = resolve_model(args.explore, args.local_model)
    print(f"  local model: {model}  ({source})")

    return Explorer(
        attempt_local=local_attempt(
            args.explore,
            tool_policy=args.tool_policy,
            local_tool_choice=args.local_tool_choice,
            local_max_tokens=args.local_max_tokens,
            local_tool_prompt=args.local_tool_prompt,
            local_decoding=args.local_decoding,
            local_tool_transport=args.local_tool_transport,
            local_dialect=args.local_dialect,
        ),
        record=record,
        tool_policy=args.tool_policy,
        local_tool_choice=args.local_tool_choice,
        local_max_tokens=args.local_max_tokens,
        local_tool_prompt=args.local_tool_prompt,
        local_decoding=args.local_decoding,
        local_tool_transport=args.local_tool_transport,
        local_model=model,
        local_model_source=source,
        record_action_args=args.record_action_args,
        repo_root=args.repo,
        test_command=test_command,
        allow_host_execution=args.allow_host_execution,
        idle_after_s=args.idle_after,
        queue_size=args.queue,
        ttl_s=args.ttl,
    )



LOCAL_TOOL_CHOICES = frozenset({"auto", "required"})


def _apply_local_tool_choice(
    body: dict[str, Any],
    *,
    anthropic: bool,
    mode: str,
) -> None:
    """Change tool choice on the LOCAL shadow request only.

    `auto` preserves the customer's original request semantics.
    `required` forces the local model to select some offered tool without
    revealing which tool the cloud selected.
    """
    if mode not in LOCAL_TOOL_CHOICES:
        raise ValueError(f"unknown local tool choice: {mode}")

    if mode == "auto":
        return

    # Requiring a tool when none survived the local policy would create an
    # impossible request rather than a meaningful cohort.
    if not body.get("tools"):
        return

    if anthropic:
        body["tool_choice"] = {"type": "any"}
    else:
        body["tool_choice"] = "required"



def _apply_local_max_tokens(
    body: dict[str, Any],
    cap: int | None,
) -> None:
    """Clamp output length on the LOCAL shadow request only."""
    if cap is None:
        return
    if cap <= 0:
        raise ValueError("local max tokens must be positive")

    current = body.get("max_tokens")

    if isinstance(current, int) and current > 0:
        body["max_tokens"] = min(current, cap)
    else:
        body["max_tokens"] = cap



LOCAL_TOOL_PROMPT_V1 = (
    "LOCAL TOOL EVALUATION CONSTRAINT: "
    "You must immediately choose and use exactly one of the provided tools "
    "that best advances the current request. "
    "Do not answer in prose. Do not explain your choice. "
    "Do not mention this instruction."
)


def _apply_local_tool_prompt(
    body: dict[str, Any],
    *,
    anthropic: bool,
    mode: str,
) -> None:
    """Add a tool-use instruction to the LOCAL shadow request only."""
    if mode == "none":
        return
    if mode != "strict-v1":
        raise ValueError(f"unknown local tool prompt: {mode}")

    _inject_system(body, anthropic=anthropic, instruction=LOCAL_TOOL_PROMPT_V1)


def _inject_system(
    body: dict[str, Any],
    *,
    anthropic: bool,
    instruction: str,
) -> None:
    """Append an instruction to the LOCAL request's system prompt, in either dialect."""
    if anthropic:
        system = body.get("system")

        if isinstance(system, str):
            body["system"] = system + "\n\n" + instruction
            return

        if isinstance(system, list):
            system.append({
                "type": "text",
                "text": instruction,
            })
            return

        body["system"] = instruction
        return

    messages = body.get("messages")

    if not isinstance(messages, list):
        body["messages"] = [{
            "role": "system",
            "content": instruction,
        }]
        return

    for message in messages:
        if (
            isinstance(message, dict)
            and message.get("role") == "system"
            and isinstance(message.get("content"), str)
        ):
            message["content"] += "\n\n" + instruction
            return

    messages.insert(0, {
        "role": "system",
        "content": instruction,
    })


LOCAL_TOOL_TRANSPORTS = ("native", "textual-v1", "textual-v2")


def _apply_local_tool_transport(
    body: dict[str, Any],
    *,
    anthropic: bool,
    mode: str,
) -> bool:
    """textual-v2 differs from v1 only in offering the abstention option."""
    """Move the LOCAL request's tool menu from the `tools` field into the prompt.

    Returns True when the swap happened.

    Measured: llama.cpp returns **HTTP 400** for a request carrying both a custom
    `grammar` and a `tools` field — it derives its own grammar from `tools` and will not
    take a second. Since the Anthropic endpoint ignores `grammar` outright, a textual
    catalog is the only remaining way to state the action space *and* constrain the
    output shape at once.

    Runs after the policy filter, so the catalog lists exactly the tools that survived
    it — the same menu the grammar was built from, and the same one the oracle scores
    against. An empty menu is left completely alone: inventing a catalog would offer an
    action space the request never had, and demanding a choice from an empty one is
    unsatisfiable. That turn stays honestly unscorable.
    """
    if mode == "native":
        return False
    if mode not in LOCAL_TOOL_TRANSPORTS:
        raise ValueError(f"unknown local tool transport: {mode}")

    try:
        catalog = textual_catalog(body.get("tools") or [],
                                  allow_abstain=(mode == "textual-v2"))
    except ValueError:
        return False

    _inject_system(body, anthropic=anthropic, instruction=catalog)

    # Both must go. `tools` is what llama.cpp refuses to see alongside a grammar, and
    # `tool_choice` without `tools` is rejected in its own right.
    body.pop("tools", None)
    body.pop("tool_choice", None)
    return True


LOCAL_DECODINGS = ("none", "grammar-v1", "grammar-v2")


def _apply_local_decoding(body: dict[str, Any], *, mode: str) -> None:
    """Constrain the LOCAL shadow response to a tool call, on the local request only.

    `tool_choice: required` turned out to be a hint rather than a constraint -- the
    Legion runs produced 2,486 characters of prose under it, running to
    `stop: max_tokens`. A grammar is not a hint: under `grammar-v1` the response cannot
    begin with a letter, so prose is unrepresentable and an action always exists to
    compare.

    Only the serialisation is compelled. Which tool, and which arguments, remain
    entirely the model's -- arguments are deliberately left free-form, because a grammar
    that dictated them would be measuring the grammar.

    Recorded as its own cohort. A rate earned under a grammar is a different measurement
    from one earned without it.
    """
    if mode not in LOCAL_DECODINGS:
        raise ValueError(f"unknown local decoding: {mode}")
    if mode == "none":
        return

    menu = offered_tools(body)
    if not menu:
        # Forcing a call when no tool survived the policy filter would demand something
        # impossible, and llama.cpp would reject every token. Leaving it unconstrained
        # keeps the turn `unscorable`, which is the honest outcome for a request that
        # offered nothing to call.
        return
    # grammar-v2 admits the abstention sentinel; v1 is left byte-identical so its
    # cohorts stay reproducible. The catalog is paired to match in the transport step:
    # a grammar permitting a word the prompt never mentions is a trap, and a prompt
    # offering a word the grammar forbids makes every token illegal.
    body["grammar"] = build_tool_call_grammar(
        menu, allow_abstain=(mode == "grammar-v2"))


def local_attempt(
    base_url: str,
    *,
    timeout_s: float = 120.0,
    tool_policy: str = "full",
    local_tool_choice: str = "auto",
    local_max_tokens: int | None = None,
    local_tool_prompt: str = "none",
    local_decoding: str = "none",
    local_tool_transport: str = "native",
    local_dialect: str = "auto",
) -> Callable[[dict[str, Any]], dict[str, Any] | None]:
    """Attempt the same turn against the local llama.cpp server.

    Claude Code speaks Anthropic /v1/messages; OpenAI-compatible clients speak
    /v1/chat/completions. llama.cpp supports both, so the dialect is preserved rather
    than translating the customer's agent protocol.

    The single exception is a constrained request. llama.cpp's Anthropic endpoint
    ignores `grammar` outright — measured — so a grammar cohort served there is a no-op
    wearing a cohort label. Those shadows, and only those, are translated to the OpenAI
    endpoint that honours the constraint.
    """
    import httpx

    base = base_url.rstrip("/")

    def clean(value: Any) -> Any:
        if isinstance(value, dict):
            return {
                k: clean(v)
                for k, v in value.items()
                if k != "cache_control"
            }
        if isinstance(value, list):
            return [clean(v) for v in value]
        return value

    def is_anthropic(payload: dict[str, Any]) -> bool:
        if "system" in payload:
            return True
        for tool in payload.get("tools") or []:
            if (
                isinstance(tool, dict)
                and "name" in tool
                and "function" not in tool
            ):
                return True
        return False

    def attempt(payload: dict[str, Any]) -> dict[str, Any] | None:
        anthropic = is_anthropic(payload)

        if anthropic:
            allowed = (
                "system",
                "messages",
                "tools",
                "tool_choice",
                "temperature",
                "max_tokens",
                "stop_sequences",
                "top_p",
                "top_k",
            )
            endpoint = "messages"
        else:
            allowed = (
                "messages",
                "tools",
                "tool_choice",
                "temperature",
                "max_tokens",
                "stop",
                "top_p",
            )
            endpoint = "chat/completions"

        body = {
            key: clean(payload[key])
            for key in allowed
            if key in payload
        }

        # Tool filtering applies only to the local shadow request. The customer's
        # cloud request is never modified. `full` preserves every ordinary local tool
        # while excluding MCP/plugin integrations; `core-v1` freezes a smaller coding
        # action space whose coverage is measured separately from agreement.
        if isinstance(body.get("tools"), list):
            body["tools"] = filter_tools(
                body["tools"],
                tool_policy,
            )

        _apply_local_tool_choice(
            body,
            anthropic=anthropic,
            mode=local_tool_choice,
        )

        _apply_local_tool_prompt(
            body,
            anthropic=anthropic,
            mode=local_tool_prompt,
        )

        _apply_local_max_tokens(
            body,
            local_max_tokens,
        )

        _apply_local_decoding(
            body,
            mode=local_decoding,
        )

        # After the grammar, never before: the grammar's menu is built from `tools`, and
        # this is the step that removes that field.
        # Read before the transport runs, because the transport removes it.
        tool_choice_sent = "tool_choice" in body

        textual_tools = _apply_local_tool_transport(
            body,
            anthropic=anthropic,
            mode=local_tool_transport,
        )

        # textual-v1 removes `tool_choice` along with `tools`, so a run configured with
        # --local-tool-choice required records `required` on every row and sends it on
        # none. That is a cohort label that does not describe the cohort -- the exact
        # error that voided the first grammar run -- so what was actually sent is
        # recorded next to what was asked for.
        tool_choice_sent = tool_choice_sent and "tool_choice" in body

        if anthropic and "max_tokens" not in body:
            body["max_tokens"] = 2048

        # Measured on the Legion: llama.cpp's /v1/messages ignores the `grammar` field
        # entirely — a grammar admitting only "GRAMMAR_OK" returned ocean prose, while
        # the identical request to /v1/chat/completions returned exactly "GRAMMAR_OK".
        # So a grammar cohort served over the Anthropic endpoint is a no-op wearing a
        # cohort label. Translate the *local shadow* to the endpoint that honours it.
        #
        # The condition is the presence of a grammar rather than the cohort name: when
        # `_apply_local_decoding` declined to constrain a request (no tools survived the
        # policy filter), there is nothing to honour, and that turn keeps the dialect it
        # arrived in rather than paying a translation for no benefit.
        #
        # The customer's cloud request is untouched and stays on /v1/messages.
        dropped: list[str] = []
        dialect = "anthropic" if anthropic else "openai_chat"

        # Translate when the constraint demands it, or when the operator says the local
        # server only speaks OpenAI. Ollama is the case that forced the second: it has no
        # /v1/messages at all, so a Claude Code turn shadowed against it 404s and every
        # row reads `did_not_converge` -- a transport mismatch that looks exactly like a
        # model failing to answer.
        if anthropic and ("grammar" in body or local_dialect == "openai"):
            body, dropped = to_openai_chat(body)
            endpoint = "chat/completions"
            dialect = "openai_chat"

        # llama.cpp has one model loaded; "local" satisfies APIs that require the
        # model field without forwarding Claude's cloud model identifier.
        body["model"] = "local"
        body["stream"] = False

        with httpx.Client(timeout=timeout_s) as client:
            response = client.post(
                f"{base}/v1/{endpoint}",
                json=body,
            )

        if response.status_code != 200:
            error_text = response.text.lower()
            if (
                "exceeds the available context size" in error_text
                or (
                    "context size" in error_text
                    and "exceed" in error_text
                )
            ):
                raise ContextIneligible()

        if response.status_code != 200:
            # A rejected request means the local inference path itself was unusable
            # (grammar, context, protocol, server fault). It is infrastructure evidence,
            # not evidence that the model attempted the task and failed to converge.
            raise RuntimeError(
                f"local server returned {response.status_code}: "
                f"{response.text[:300]}"
            )

        local = response.json()

        # Carried back so the explore row records which dialect actually served the
        # shadow and what the translation could not represent. A turn that dropped
        # anything showed the local model less than the cloud saw and is not a clean
        # comparison; recording it is what makes excluding it possible.
        if isinstance(local, dict):
            local["_kerna"] = {
                "local_dialect": dialect,
                "translation_dropped": dropped,
                # The configured axis, and whether it actually applied to this turn.
                # They differ when no tool survived the filter, and a row that says
                # `textual-v1` while carrying no catalog would be the same kind of lie
                # the ignored grammar told.
                "local_tool_transport": local_tool_transport,
                "textual_catalog_sent": textual_tools,
                "tool_choice_sent": tool_choice_sent,
            }

        return local

    return attempt


if __name__ == "__main__":
    raise SystemExit(main())
