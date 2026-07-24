"""Shared, local-only plumbing for the tau3-to-Kerna gateway adapter.

This module has no provider dependency.  It exposes a supplied tool list and
handler over an authenticated loopback HTTP endpoint, launches ``kerna
gateway`` with the stdio MCP bridge, and offers a small synchronous MCP client
to the tau3 runner and contract test.
"""

from __future__ import annotations

import json
import os
import re
import secrets
import subprocess
import sys
import threading
import time
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any, Callable


REPO_ROOT = Path(__file__).resolve().parents[2]
BRIDGE_PATH = REPO_ROOT / "benchmarks" / "tau3" / "tau3_bridge.py"


class AdapterError(RuntimeError):
    """A local adapter or MCP protocol failure."""


class LoopbackBridgeServer:
    """Authenticated localhost-only endpoint owned by the tau3 parent."""

    def __init__(
        self,
        tools: Callable[[], list[dict[str, Any]]],
        call_tool: Callable[[str, dict[str, Any]], dict[str, Any]],
    ) -> None:
        self._tools = tools
        self._call_tool = call_tool
        self.token = secrets.token_urlsafe(32)
        outer = self

        class Handler(BaseHTTPRequestHandler):
            def do_POST(self) -> None:  # noqa: N802 - BaseHTTPRequestHandler API
                if self.headers.get("Authorization") != f"Bearer {outer.token}":
                    self.send_error(401)
                    return
                try:
                    length = int(self.headers.get("Content-Length", "0"))
                    payload = json.loads(self.rfile.read(length).decode("utf-8")) if length else {}
                    if self.path == "/tools":
                        response = {"tools": outer._tools()}
                    elif self.path == "/call":
                        name = payload.get("name")
                        arguments = payload.get("arguments")
                        if not isinstance(name, str) or not isinstance(arguments, dict):
                            raise AdapterError("tool call requires a string name and object arguments")
                        response = {"toolMessage": outer._call_tool(name, arguments)}
                    else:
                        self.send_error(404)
                        return
                    encoded = json.dumps(response).encode("utf-8")
                    self.send_response(200)
                    self.send_header("Content-Type", "application/json")
                    self.send_header("Content-Length", str(len(encoded)))
                    self.end_headers()
                    self.wfile.write(encoded)
                except Exception as exc:
                    body = json.dumps({"error": str(exc)}).encode("utf-8")
                    self.send_response(500)
                    self.send_header("Content-Type", "application/json")
                    self.send_header("Content-Length", str(len(body)))
                    self.end_headers()
                    self.wfile.write(body)

            def log_message(self, _format: str, *_args: Any) -> None:
                return

        self._server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.endpoint = f"http://127.0.0.1:{self._server.server_address[1]}"
        self._thread = threading.Thread(target=self._server.serve_forever, daemon=True)

    def __enter__(self) -> "LoopbackBridgeServer":
        self._thread.start()
        return self

    def __exit__(self, *_: Any) -> None:
        self._server.shutdown()
        self._server.server_close()
        self._thread.join(timeout=5)


@dataclass
class GatewayClient:
    process: subprocess.Popen[str]
    stderr_lines: list[str] = field(default_factory=list)
    _next_id: int = 1

    @classmethod
    def start(cls, *, kerna: Path, run_dir: Path, endpoint: str, token: str, allowed_tools: list[str]) -> "GatewayClient":
        run_dir.mkdir(parents=True, exist_ok=True)
        config = run_dir / "kerna.toml"
        bridge_args = [str(BRIDGE_PATH), "--endpoint", endpoint, "--token", token]
        toml_args = ", ".join(json.dumps(value) for value in bridge_args)
        permissions = "\n\n".join(
            f'[[permissions]]\ntool = {json.dumps(tool)}\naction = "auto_approve"' for tool in allowed_tools
        )
        config.write_text(
            "\n".join(
                [
                    'llm_provider = "mock"',
                    'llm_model = "gateway-only"',
                    'db_path = "kerna-gateway.db"',
                    'sandbox_dir = "sandbox"',
                    'memory_backend = "sqlite"',
                    'runtime_mode = "native"',
                    'network_mode = "none"',
                    '',
                    '[[mcp_servers]]',
                    'name = "tau3-loopback"',
                    f'command = {json.dumps(sys.executable)}',
                    f"args = [{toml_args}]",
                    'allow_tools = [' + ", ".join(json.dumps(tool) for tool in allowed_tools) + ']',
                    '',
                    permissions,
                    '',
                ]
            ),
            encoding="utf-8",
        )
        # Kerna's gateway does not need a provider.  Do not pass the benchmark
        # caller's API credential to it or to the bridge child.
        environment = os.environ.copy()
        environment.pop("OPENAI_API_KEY", None)
        environment.pop("KERNA_LLM_API_KEY", None)
        process = subprocess.Popen(
            [str(kerna), "gateway"],
            cwd=run_dir,
            env=environment,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            bufsize=1,
        )
        client = cls(process)
        assert process.stderr is not None
        threading.Thread(target=client._collect_stderr, daemon=True).start()
        client.initialize()
        return client

    def _collect_stderr(self) -> None:
        assert self.process.stderr is not None
        for line in self.process.stderr:
            self.stderr_lines.append(line.rstrip())

    @property
    def task_id(self) -> str | None:
        pattern = re.compile(r"task ([0-9a-f-]{36})")
        for line in self.stderr_lines:
            match = pattern.search(line)
            if match:
                return match.group(1)
        return None

    def _request(self, method: str, params: dict[str, Any]) -> dict[str, Any]:
        if self.process.poll() is not None:
            raise AdapterError("kerna gateway exited before responding: " + "\n".join(self.stderr_lines[-10:]))
        request_id = self._next_id
        self._next_id += 1
        assert self.process.stdin is not None and self.process.stdout is not None
        self.process.stdin.write(json.dumps({"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}) + "\n")
        self.process.stdin.flush()
        deadline = time.monotonic() + 35
        while time.monotonic() < deadline:
            line = self.process.stdout.readline()
            if not line:
                break
            try:
                message = json.loads(line)
            except json.JSONDecodeError:
                continue
            if message.get("id") == request_id:
                if "error" in message:
                    raise AdapterError(str(message["error"]))
                return message["result"]
        raise AdapterError("timed out waiting for Kerna gateway: " + "\n".join(self.stderr_lines[-10:]))

    def initialize(self) -> None:
        self._request(
            "initialize",
            {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "kerna-tau3-adapter", "version": "0.1.0"}},
        )

    def list_tools(self) -> list[dict[str, Any]]:
        return self._request("tools/list", {}).get("tools", [])

    def call_tool(self, name: str, arguments: dict[str, Any]) -> dict[str, Any]:
        return self._request("tools/call", {"name": name, "arguments": arguments})

    def close(self) -> None:
        if self.process.stdin is not None and not self.process.stdin.closed:
            self.process.stdin.close()
        try:
            self.process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait(timeout=5)

