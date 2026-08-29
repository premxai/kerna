"""Observation mode — sit in the path, change nothing, write down what goes by.

    python observe.py --upstream https://api.anthropic.com/v1
    ANTHROPIC_BASE_URL=http://127.0.0.1:8127 claude

Then work normally for an hour and read the report:

    python observe.py --report

Every routing rule in this package was designed against an imagined traffic shape. This
records the real one. It **never routes locally and never modifies a response** — it is a
transparent relay whose only side effect is a local JSONL file.

## Both API shapes

`/v1/chat/completions` (OpenAI: Cursor, Continue, aider) and `/v1/messages` (Anthropic:
Claude Code). The path is preserved when forwarding, so the upstream sees exactly what the
client sent.

## Streaming is relayed, never buffered

A proxy that buffers a stream to inspect it turns a responsive tool into one that looks
frozen. Chunks go straight through as they arrive; only their size and count are noted.

## The three questions this answers

1. **Does real traffic stream?** The dispatcher currently refuses to route any streaming
   request. If everything streams, that rule excludes all traffic and must change.
2. **Do we ever see the gate outcome?** If an agent sends its own test results back as
   `tool_result` blocks, the cascade gets its verification signal for free. If not, the
   ledger needs a different source and that is a significant design change.
3. **What task classes exist?** Earned autonomy is per class, and a classifier cannot be
   designed against imagination.
"""

from __future__ import annotations

import json
import platform
import time
from collections import Counter
from pathlib import Path
from typing import Any

from .interceptor import DEFAULT_PORT, forwardable_headers, stream_passthrough
from . import correlate
from .recorder import TrafficRecorder

DEFAULT_LOG = Path("evals/traffic.jsonl")

# Endpoints we relay. Anything else gets a plain 404 rather than a guess — silently
# forwarding an unknown path would make us responsible for behaviour we never tested.
ROUTES = {
    "/v1/chat/completions": "chat/completions",
    "/v1/messages": "messages",
}


def build_observer(*, upstream: str, recorder: TrafficRecorder, quiet: bool = False):
    from http.server import BaseHTTPRequestHandler

    import httpx

    base = upstream.rstrip("/")
    client = httpx.Client(timeout=600.0)

    class Observer(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, fmt: str, *args) -> None:  # noqa: A003
            return  # our own one-line summary is more useful than the access log

        def handle_one_request(self) -> None:
            """Swallow the disconnects that are normal, not faults.

            A client that quits, times out, or abandons a stream resets the connection,
            and the stdlib prints a full traceback for each one. During a real session
            that fills the window with alarming red text about a condition that is
            completely expected — and the operator, reasonably, assumes the tool is
            broken and stops the experiment. This happened.
            """
            try:
                super().handle_one_request()
            except (ConnectionResetError, ConnectionAbortedError, BrokenPipeError):
                self.close_connection = True

        def _json(self, body: dict[str, Any], status: int) -> None:
            encoded = json.dumps(body).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(encoded)))
            self.end_headers()
            self.wfile.write(encoded)

        def _relay_other(self, method: str) -> None:
            """Relay any path we do not specifically understand, unchanged.

            Observation mode never routes locally, so a transparent proxy must pass
            everything through — including endpoints we have never heard of. Refusing
            unknown paths seemed prudent and was actively wrong: Claude Code validates
            the selected model before its first request, our 404 was read as "that model
            does not exist", and the session failed with zero requests recorded. A relay
            that answers some paths and not others is not a relay.
            """
            length = int(self.headers.get("Content-Length") or 0)
            body = self.rfile.read(length) if length else None
            headers = forwardable_headers({k: v for k, v in self.headers.items()})
            try:
                response = client.request(
                    method, f"{base}{self.path.replace('/v1', '', 1)}"
                    if self.path.startswith("/v1/") else f"{base}{self.path}",
                    content=body, headers=headers,
                )
                payload = response.content
                self.send_response(response.status_code)
                self.send_header("Content-Type",
                                 response.headers.get("content-type", "application/json"))
                self.send_header("Content-Length", str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)
                if not quiet:
                    print(f"  {method:<7}{self.path:<34}-> {response.status_code}")
            except Exception as exc:  # noqa: BLE001
                self._json({"error": {"message": f"relay failed: {type(exc).__name__}"}}, 502)

        def do_GET(self) -> None:  # noqa: N802
            if self.path.rstrip("/") in ("/health", "/v1/health"):
                self._json({"status": "ok", "mode": "observe",
                            "recorded": recorder.count}, 200)
                return
            self._relay_other("GET")

        def do_POST(self) -> None:  # noqa: N802
            endpoint = ROUTES.get(self.path.rstrip("/").split("?")[0])
            if endpoint is None:
                self._relay_other("POST")
                return

            length = int(self.headers.get("Content-Length") or 0)
            raw = self.rfile.read(length) if length else b"{}"
            headers = forwardable_headers({k: v for k, v in self.headers.items()})

            try:
                payload = json.loads(raw or b"{}")
            except ValueError:
                self._json({"error": {"message": "body was not valid JSON"}}, 400)
                return

            started = time.monotonic()

            # One id for this turn, inherited when a caller supplied one. Minted here
            # rather than at each writer, so a request and its error row share a key.
            this_turn = correlate.turn_id(headers)
            streaming = bool(payload.get("stream"))

            try:
                if streaming:
                    # Headers must go out BEFORE the first chunk, and without a
                    # Content-Length: the body length is unknown and declaring one
                    # truncates the stream.
                    #
                    # `Connection: close` is load-bearing, not tidiness. Under HTTP/1.1
                    # a body with neither Content-Length nor chunked encoding is
                    # terminated by the connection closing; declaring keep-alive instead
                    # leaves the client waiting forever for bytes that never come. The
                    # editor hangs, and it looks like the model is slow.
                    self.send_response(200)
                    self.send_header("Content-Type", "text/event-stream")
                    self.send_header("Cache-Control", "no-cache")
                    self.send_header("Connection", "close")
                    self.end_headers()
                    self.close_connection = True

                    def write(chunk: bytes) -> None:
                        self.wfile.write(chunk)
                        self.wfile.flush()   # or the client sees nothing until the end

                    status, meta = stream_passthrough(
                        base, endpoint, payload, headers, write
                    )
                else:
                    response = client.post(
                        f"{base}/{endpoint}", json=payload, headers=headers
                    )
                    status = response.status_code
                    body = response.content
                    meta = {"bytes": len(body)}
                    try:
                        usage = response.json().get("usage")
                        if usage:
                            meta["usage"] = usage
                    except ValueError:
                        pass
                    self.send_response(status)
                    self.send_header(
                        "Content-Type",
                        response.headers.get("content-type", "application/json"),
                    )
                    self.send_header("Content-Length", str(len(body)))
                    self.end_headers()
                    self.wfile.write(body)
            except Exception as exc:  # noqa: BLE001
                elapsed = (time.monotonic() - started) * 1000.0
                recorder.record(endpoint, payload, route="error", reason=type(exc).__name__,
                                status=502, elapsed_ms=elapsed, turn=this_turn)
                if not streaming:
                    self._json({"error": {"message": f"relay failed: {type(exc).__name__}"}}, 502)
                return

            elapsed = (time.monotonic() - started) * 1000.0
            recorder.record(endpoint, payload, route="observe", reason="passthrough",
                            status=status, elapsed_ms=elapsed, response_meta=meta,
                            headers=headers, turn=this_turn)

            if not quiet:
                n = len(payload.get("messages") or [])
                tools = len(payload.get("tools") or [])
                flags = "stream" if streaming else "block"
                # An upstream error is the single most useful thing to see here. A bare
                # timing line for a 429 reads as "the relay is broken" when the real
                # answer is "your account is rate limited" -- a distinction that cost a
                # whole session to work out.
                warn = "" if 200 <= status < 300 else f"  <-- UPSTREAM {status}"
                print(f"  {endpoint:<16}{flags:<7}{n:>3} msgs {tools:>3} tools "
                      f"{elapsed:>7.0f} ms  [{recorder.count}]{warn}")
                if status == 429:
                    print("      429 = rate limited or out of credit. Try /model inside")
                    print("      Claude and pick a cheaper model (Haiku, Sonnet).")

    return Observer


def serve(*, upstream: str, log: Path, port: int, with_content: bool, quiet: bool) -> None:
    from http.server import ThreadingHTTPServer

    recorder = TrafficRecorder(path=log, with_content=with_content)
    server = ThreadingHTTPServer(("127.0.0.1", port), build_observer(
        upstream=upstream, recorder=recorder, quiet=quiet))

    print(f"observing on http://127.0.0.1:{port}  ->  {upstream}")
    print(f"  recording to {log}"
          f"{'  (WITH prompt samples)' if with_content else '  (structure only)'}")
    print("  nothing is routed locally and no response is modified.")
    print()
    print("  In a SECOND terminal, point a tool at it:")
    print()
    # The shell matters. `VAR=value command` is bash syntax and is a hard error in
    # PowerShell, which is the default terminal on Windows — printing only the bash form
    # sends a Windows user straight into CommandNotFoundException.
    if platform.system() == "Windows":
        print("    PowerShell:")
        print(f'      $env:ANTHROPIC_BASE_URL = "http://127.0.0.1:{port}"')
        print("      claude")
        print()
        print("    Git Bash:")
        print(f"      ANTHROPIC_BASE_URL=http://127.0.0.1:{port} claude")
    else:
        print(f"    ANTHROPIC_BASE_URL=http://127.0.0.1:{port} claude")
    print()
    print("    For Cursor / Continue / aider, set instead:")
    print(f"      OPENAI_BASE_URL = http://127.0.0.1:{port}/v1")
    print()
    print("  Then work normally. Stop with Ctrl-C and run:  python observe.py --report")
    print()
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print(f"\nstopped. {recorder.count} requests recorded to {log}")
    finally:
        server.server_close()


# ------------------------------------------------------------------ report


def report(log: Path, *, all_runs: bool = False) -> str:
    """Summarise the most recent run, or every run with `all_runs`.

    Defaulting to the latest run is not a convenience. Averaging across runs once mixed
    a relay bug with its own fix and reported "57% streaming", a figure that described
    no session that had ever taken place. Comparisons need runs kept apart.
    """
    rows: list[dict] = []
    runs = 0
    for line in log.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            row = json.loads(line)
        except json.JSONDecodeError:
            continue
        if row.get("record") == "header":
            runs += 1
            if not all_runs:
                rows = []          # a new run supersedes what came before
        elif row.get("record") == "request":
            rows.append(row)

    if not rows:
        return f"no requests recorded in {log}"

    scope = (f"all {runs} runs" if all_runs else
             f"most recent of {runs} runs (use --all-runs for everything)")

    n = len(rows)
    streaming = sum(1 for r in rows if r.get("stream"))
    with_tools = sum(1 for r in rows if r.get("n_tools", 0) > 0)
    with_results = sum(1 for r in rows if r.get("has_tool_results"))
    endpoints = Counter(r.get("endpoint") for r in rows)
    models = Counter(r.get("model") for r in rows)
    blocks: Counter = Counter()
    tools: Counter = Counter()
    for r in rows:
        blocks.update(r.get("block_types") or {})
        tools.update(r.get("tool_names") or [])

    msgs = sorted(r.get("n_messages", 0) for r in rows)
    chars = sorted(r.get("total_chars", 0) for r in rows)

    def pct(k: int) -> str:
        return f"{k / n:.0%}"

    def median(xs: list[int]) -> int:
        return xs[len(xs) // 2] if xs else 0

    out = [
        f"# Real traffic — {n} requests · {scope}",
        "",
        "## Q1. Does real traffic stream?",
        f"  streaming: {streaming}/{n} ({pct(streaming)})",
        "",
        "  The dispatcher currently refuses to route ANY streaming request "
        "(Decision 031).",
    ]
    if streaming == n:
        out.append("  -> every request streams. The rule as written routes NOTHING and")
        out.append("     must be replaced by a signal that separates 'a human is waiting'")
        out.append("     from 'this is how agents talk'.")
    elif streaming == 0:
        out.append("  -> nothing streams. The rule costs nothing as written.")
    else:
        out.append(f"  -> the rule would exclude {pct(streaming)} of traffic.")

    out += [
        "",
        "## Q2. Do we ever see the gate outcome?",
        f"  requests carrying tool_result blocks: {with_results}/{n} ({pct(with_results)})",
        f"  requests declaring tools:             {with_tools}/{n} ({pct(with_tools)})",
        "",
    ]
    if with_results:
        out.append("  -> the agent feeds its own tool output back. Test results are")
        out.append("     visible in the conversation, so the cascade can read the gate")
        out.append("     outcome for free rather than needing to run tests itself.")
    else:
        out.append("  -> no tool results observed. The gate outcome is NOT visible from")
        out.append("     this position, and earned autonomy needs another signal source.")

    out += [
        "",
        "## Q3. What does the traffic look like?",
        f"  endpoints:      {dict(endpoints)}",
        f"  models:         {dict(models.most_common(5))}",
        f"  messages/req:   median {median(msgs)}, max {max(msgs) if msgs else 0}",
        f"  chars/req:      median {median(chars):,}, max {max(chars) if chars else 0:,}",
        f"  block types:    {dict(blocks.most_common(8))}",
    ]
    if tools:
        out.append(f"  tools declared: {', '.join(t for t, _ in tools.most_common(12))}")
    return "\n".join(out)


def main(argv: list[str] | None = None) -> int:
    import argparse

    parser = argparse.ArgumentParser(
        prog="observe",
        description="Relay traffic unchanged and record its shape.",
    )
    parser.add_argument("--upstream", default="https://api.anthropic.com/v1")
    parser.add_argument("--port", type=int, default=DEFAULT_PORT)
    parser.add_argument("--log", default=str(DEFAULT_LOG))
    parser.add_argument("--record-content", action="store_true",
                        help="also sample prompt text (off by default: a recording that "
                             "quietly contains proprietary source is a liability)")
    parser.add_argument("--report", action="store_true", help="summarise an existing log")
    parser.add_argument("--all-runs", action="store_true",
                        help="report across every run rather than only the most recent")
    parser.add_argument("--quiet", action="store_true")
    args = parser.parse_args(argv)

    log = Path(args.log)
    if args.report:
        if not log.exists():
            print(f"no log at {log} — run the observer first")
            return 2
        print(report(log, all_runs=args.all_runs))
        return 0

    serve(upstream=args.upstream, log=log, port=args.port,
          with_content=args.record_content, quiet=args.quiet)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
