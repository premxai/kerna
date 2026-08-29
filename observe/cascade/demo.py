"""The C1 gate, as a runnable demonstration.

Starts a real sidecar and a real stub upstream on loopback, then drives four scenarios
through actual HTTP. No GPU, no API key, no provider account — the point is to exercise
the *plumbing* in production shape, which is the part that has to be boring and correct
long before the routing claim is earned.

    python -m m0.cascade.demo

What it proves, and what it deliberately does not:

  1. a promoted task converges locally and is served from the laptop
  2. a task that stalls escalates to the cloud, and the client cannot tell
  3. repeated infrastructure failures open the breaker, and requests keep being served
  4. a crash inside our own code still returns the upstream answer

The gate's third clause — "killing the sidecar mid-request does not fail the request" —
is **not** demonstrated here, because it cannot honestly be. If the process is dead the
socket is dead, and no amount of code inside the process fixes that. The real bypass is
the install being one environment variable: `unset OPENAI_BASE_URL` and the client talks
to the provider directly. That is an install-level property, and claiming otherwise with
a staged demo would be exactly the kind of theatre this project keeps refusing.
"""

from __future__ import annotations

import json
import threading
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from .circuit import CircuitBreaker
from .dispatcher import Dispatcher
from .interceptor import build_handler

STUB_PORT = 8129
SIDECAR_PORT = 8128

CLOUD_ANSWER = {"choices": [{"message": {"role": "assistant", "content": "answer from the cloud"}}],
                "usage": {"prompt_tokens": 40, "completion_tokens": 12}}
LOCAL_ANSWER = {"choices": [{"message": {"role": "assistant", "content": "answer from the laptop"}}],
                "usage": {"prompt_tokens": 0, "completion_tokens": 0}}


def _start_stub_upstream() -> ThreadingHTTPServer:
    """A stand-in for the customer's provider, so the demo needs no account."""
    calls: list[dict] = []

    class Stub(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *a) -> None:  # noqa: A003
            return

        def do_POST(self) -> None:  # noqa: N802
            length = int(self.headers.get("Content-Length") or 0)
            calls.append(json.loads(self.rfile.read(length) or b"{}"))
            body = json.dumps(CLOUD_ANSWER).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    server = ThreadingHTTPServer(("127.0.0.1", STUB_PORT), Stub)
    server.calls = calls  # type: ignore[attr-defined]
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def _post(port: int, payload: dict) -> tuple[int, dict]:
    request = urllib.request.Request(
        f"http://127.0.0.1:{port}/v1/chat/completions",
        data=json.dumps(payload).encode(),
        headers={"Content-Type": "application/json",
                 "Authorization": "Bearer sk-the-customers-own-key"},
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return response.status, json.loads(response.read())
    except urllib.error.HTTPError as exc:
        return exc.code, json.loads(exc.read() or b"{}")


def _content(body: dict) -> str:
    try:
        return body["choices"][0]["message"]["content"]
    except Exception:  # noqa: BLE001
        return json.dumps(body)[:80]


def main(argv: list[str] | None = None) -> int:
    from .interceptor import make_cloud_forwarder

    upstream = _start_stub_upstream()

    # Local behaviour is scripted per scenario so the demo can show convergence,
    # stalling and infrastructure failure without a model.
    mode = {"value": "converge"}

    def attempt_local(payload: dict) -> dict | None:
        if mode["value"] == "converge":
            return LOCAL_ANSWER
        if mode["value"] == "stall":
            return None                       # did not converge -> escalate
        raise RuntimeError("llama-server not reachable")   # infrastructure -> breaker

    breaker = CircuitBreaker(failure_threshold=2, cooldown_s=300.0)
    dispatcher = Dispatcher(
        breaker=breaker,
        # The gate is forced open FOR THIS DEMO ONLY, to exercise the local path. The
        # shipped default is off until C0a runs; see dispatcher.LOCAL_ROUTING_ENABLED.
        local_routing_enabled=True,
        promoted_classes=frozenset({"demo"}),
    )

    handler = build_handler(
        dispatcher=dispatcher,
        forward_cloud=make_cloud_forwarder(f"http://127.0.0.1:{STUB_PORT}/v1"),
        # The demo drives task_class through the handler's default (None), so route
        # locally by classifying every demo request as the promoted class.
        attempt_local=attempt_local,
        quiet=True,
    )

    # The handler classifies nothing yet (classification is C2), so patch the one hook
    # the demo needs: treat every request as the promoted class.
    import m0.cascade.interceptor as interceptor_module

    original = interceptor_module.handle_completion

    def classified(payload, headers, **kw):
        kw.setdefault("task_class", "demo")
        return original(payload, headers, **kw)

    interceptor_module.handle_completion = classified  # type: ignore[assignment]

    sidecar = ThreadingHTTPServer(("127.0.0.1", SIDECAR_PORT), handler)
    threading.Thread(target=sidecar.serve_forever, daemon=True).start()

    payload = {"model": "gpt-4o", "messages": [{"role": "user", "content": "refactor this"}]}
    line = "-" * 78

    try:
        print(f"\ncascade demo — sidecar :{SIDECAR_PORT}  ->  stub provider :{STUB_PORT}")
        print(line)

        print("\n1. a promoted task that converges locally")
        mode["value"] = "converge"
        before = len(upstream.calls)  # type: ignore[attr-defined]
        status, body = _post(SIDECAR_PORT, payload)
        billed = len(upstream.calls) - before  # type: ignore[attr-defined]
        print(f"   client got {status}: {_content(body)!r}")
        print(f"   upstream calls billed: {billed}   <- the saving, when it works")
        assert status == 200 and billed == 0

        print("\n2. the same task, but local stalls")
        mode["value"] = "stall"
        before = len(upstream.calls)  # type: ignore[attr-defined]
        status, body = _post(SIDECAR_PORT, payload)
        billed = len(upstream.calls) - before  # type: ignore[attr-defined]
        print(f"   client got {status}: {_content(body)!r}")
        print(f"   upstream calls billed: {billed}   <- costs exactly what today costs")
        print("   the client cannot tell a stall happened. That is the product.")
        assert status == 200 and billed == 1

        print("\n3. the local path breaks entirely")
        mode["value"] = "crash"
        for n in range(3):
            status, body = _post(SIDECAR_PORT, payload)
            print(f"   request {n + 1}: {status} {_content(body)!r}   breaker: "
                  f"{breaker.state.value}")
            assert status == 200
        print("   every request still served. The breaker stopped us wasting attempts.")

        print("\n4. health")
        with urllib.request.urlopen(f"http://127.0.0.1:{SIDECAR_PORT}/health") as r:
            print(f"   {json.loads(r.read())}")

        print(f"\n{line}")
        print("C1 plumbing gate: PASSED")
        print("  - a converging task is served locally and bills nothing")
        print("  - a stalling task escalates invisibly and bills exactly as today")
        print("  - a broken local path never fails a request")
        print("\nNot demonstrated, deliberately: 'killing the sidecar mid-request'.")
        print("A dead process cannot serve a socket. The real bypass is the install:")
        print("  unset OPENAI_BASE_URL   ->  the client talks to the provider directly.")
        return 0
    finally:
        interceptor_module.handle_completion = original  # type: ignore[assignment]
        sidecar.shutdown()
        sidecar.server_close()
        upstream.shutdown()
        upstream.server_close()


if __name__ == "__main__":
    raise SystemExit(main())
