#!/usr/bin/env python3
"""Schema-preserving ToolEmu-to-Kerna gateway transport.

The parent process owns the selected ToolEmu case and the emulator callback.
Kerna owns discovery, policy, budgets, and receipts. The stdio child owns no
ToolEmu state and can only reach its authenticated localhost parent.
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
BRIDGE_PATH = Path(__file__).with_name("toolemu_bridge.py")
TYPE_MAP = {"integer": "integer", "number": "number", "string": "string", "boolean": "boolean", "array": "array", "object": "object"}


class AdapterError(RuntimeError):
    """A local adapter or JSON-RPC protocol failure."""


def mcp_tool_name(toolkit: str, tool: str) -> str:
    """Make a stable, OpenAI-compatible name without losing the source mapping."""
    clean = lambda value: re.sub(r"[^A-Za-z0-9_-]+", "_", value).strip("_").lower()
    return f"toolemu__{clean(toolkit)}__{clean(tool)}"


def tool_schema(toolkit: dict[str, Any], tool: dict[str, Any]) -> dict[str, Any]:
    properties = {
        item["name"]: {"type": TYPE_MAP.get(item["type"], "string"), "description": item.get("description", "")}
        for item in tool.get("parameters", [])
    }
    required = [item["name"] for item in tool.get("parameters", []) if item.get("required", False)]
    return {
        "name": mcp_tool_name(toolkit["toolkit"], tool["name"]),
        "description": f"[ToolEmu {toolkit['toolkit']}.{tool['name']}] {tool.get('summary', '')}",
        "inputSchema": {"type": "object", "properties": properties, "required": required, "additionalProperties": False},
        "_toolemu": {"toolkit": toolkit["toolkit"], "tool": tool["name"], "returns": tool.get("returns", [])},
    }


class ToolEmuCaseEnvironment:
    """One selected ToolEmu case and an authoritative observation callback."""

    def __init__(self, case: dict[str, Any], toolkits: list[dict[str, Any]], observe: Callable[[dict[str, Any]], Any]) -> None:
        self.case = case
        selected = set(case["Toolkits"])
        self.toolkits = [toolkit for toolkit in toolkits if toolkit["toolkit"] in selected]
        if {toolkit["toolkit"] for toolkit in self.toolkits} != selected:
            raise AdapterError("the selected case references a missing ToolEmu toolkit")
        self._schemas = [tool_schema(toolkit, tool) for toolkit in self.toolkits for tool in toolkit["tools"]]
        self._schema_by_name = {schema["name"]: schema for schema in self._schemas}
        self._observe = observe
        self.trace: list[dict[str, Any]] = []

    @classmethod
    def from_assets(cls, assets: Path, case_name: str, observe: Callable[[dict[str, Any]], Any]) -> "ToolEmuCaseEnvironment":
        cases = json.loads((assets / "all_cases.json").read_text(encoding="utf-8"))
        case = next((item for item in cases if item.get("name") == case_name), None)
        if case is None:
            raise AdapterError(f"ToolEmu case not found: {case_name}")
        toolkits = json.loads((assets / "all_toolkits.json").read_text(encoding="utf-8"))
        return cls(case, toolkits, observe)

    def tools(self) -> list[dict[str, Any]]:
        return [{key: value for key, value in schema.items() if key != "_toolemu"} for schema in self._schemas]

    def call(self, name: str, arguments: dict[str, Any]) -> Any:
        schema = self._schema_by_name.get(name)
        if schema is None:
            raise AdapterError(f"unregistered ToolEmu MCP tool: {name}")
        source = schema["_toolemu"]
        request = {
            "case": self.case,
            "toolkit": source["toolkit"],
            "tool": source["tool"],
            "arguments": arguments,
            "toolEmuReturns": source["returns"],
        }
        observation = self._observe(request)
        self.trace.append({"mcpTool": name, "toolkit": source["toolkit"], "tool": source["tool"], "arguments": arguments, "observation": observation})
        return observation


class LoopbackBridgeServer:
    """Authenticated localhost parent endpoint for one ToolEmu case."""

    def __init__(self, environment: ToolEmuCaseEnvironment) -> None:
        self.environment = environment
        self.token = secrets.token_urlsafe(32)
        outer = self

        class Handler(BaseHTTPRequestHandler):
            def do_POST(self) -> None:  # noqa: N802 - stdlib API
                if self.headers.get("Authorization") != f"Bearer {outer.token}":
                    self.send_error(401)
                    return
                try:
                    length = int(self.headers.get("Content-Length", "0"))
                    payload = json.loads(self.rfile.read(length).decode("utf-8")) if length else {}
                    if self.path == "/tools":
                        response = {"tools": outer.environment.tools()}
                    elif self.path == "/call":
                        name, arguments = payload.get("name"), payload.get("arguments")
                        if not isinstance(name, str) or not isinstance(arguments, dict):
                            raise AdapterError("tool call requires a string name and object arguments")
                        response = {"observation": outer.environment.call(name, arguments), "isError": False}
                    else:
                        self.send_error(404)
                        return
                    body = json.dumps(response).encode("utf-8")
                    self.send_response(200)
                    self.send_header("Content-Type", "application/json")
                    self.send_header("Content-Length", str(len(body)))
                    self.end_headers()
                    self.wfile.write(body)
                except Exception as error:
                    body = json.dumps({"error": str(error)}).encode("utf-8")
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
        bridge_args = [str(BRIDGE_PATH), "--endpoint", endpoint, "--token", token]
        permissions = "\n\n".join(f'[[permissions]]\ntool = {json.dumps(tool)}\naction = "auto_approve"' for tool in allowed_tools)
        (run_dir / "kerna.toml").write_text(
            "\n".join([
                'llm_provider = "mock"', 'llm_model = "gateway-only"', 'db_path = "kerna-gateway.db"',
                'sandbox_dir = "sandbox"', 'memory_backend = "sqlite"', 'runtime_mode = "native"', 'network_mode = "none"', '',
                '[[mcp_servers]]', 'name = "toolemu-loopback"', f'command = {json.dumps(sys.executable)}',
                "args = [" + ", ".join(json.dumps(value) for value in bridge_args) + "]",
                "allow_tools = [" + ", ".join(json.dumps(tool) for tool in allowed_tools) + "]", '', permissions, '',
            ]),
            encoding="utf-8",
        )
        environment = os.environ.copy()
        environment.pop("OPENAI_API_KEY", None)
        environment.pop("KERNA_LLM_API_KEY", None)
        process = subprocess.Popen([str(kerna), "gateway"], cwd=run_dir, env=environment, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, encoding="utf-8", bufsize=1)
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
        for line in self.stderr_lines:
            match = re.search(r"task ([0-9a-f-]{36})", line)
            if match:
                return match.group(1)
        return None

    def _request(self, method: str, params: dict[str, Any]) -> dict[str, Any]:
        if self.process.poll() is not None:
            raise AdapterError("kerna gateway exited: " + "\n".join(self.stderr_lines[-10:]))
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
        self._request("initialize", {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "kerna-toolemu-adapter", "version": "0.1.0"}})

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
