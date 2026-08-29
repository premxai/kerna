#!/usr/bin/env python3
"""Minimal stdio MCP filesystem server used for Kerna containment acceptance.

It has no third-party dependencies and only accepts paths below the mounted
`/workspace/read` and `/workspace/write` directories. Kerna's Docker runner is
still the security boundary; these checks make the fixture's intent explicit.
"""

import json
import socket
import sys
from pathlib import Path

READ_ROOT = Path("/workspace/read").resolve()
WRITE_ROOT = Path("/workspace/write").resolve()


def respond(request_id, result=None, error=None):
    message = {"jsonrpc": "2.0", "id": request_id}
    if error is not None:
        message["error"] = error
    else:
        message["result"] = result
    print(json.dumps(message), flush=True)


def safe_path(root, value):
    candidate = (root / value).resolve()
    if candidate == root or root in candidate.parents:
        return candidate
    raise ValueError("path is outside the permitted mounted root")


def tools():
    return [
        {
            "name": "read_file",
            "description": "Read a file below the read-only /workspace/read mount.",
            "inputSchema": {"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]},
        },
        {
            "name": "write_file",
            "description": "Write a file below the writable /workspace/write mount.",
            "inputSchema": {"type": "object", "properties": {"path": {"type": "string"}, "content": {"type": "string"}}, "required": ["path", "content"]},
        },
        {
            "name": "network_probe",
            "description": "Attempt a TCP connection; it must fail under Kerna's network=none runtime.",
            "inputSchema": {"type": "object", "properties": {}},
        },
    ]


def call(name, arguments):
    if name == "read_file":
        value = safe_path(READ_ROOT, arguments["path"]).read_text(encoding="utf-8")
        return {"content": [{"type": "text", "text": value}]}
    if name == "write_file":
        destination = safe_path(WRITE_ROOT, arguments["path"])
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(arguments["content"], encoding="utf-8")
        return {"content": [{"type": "text", "text": "written"}]}
    if name == "network_probe":
        try:
            with socket.create_connection(("1.1.1.1", 53), timeout=2):
                return {"content": [{"type": "text", "text": "network unexpectedly available"}], "isError": True}
        except OSError as exc:
            return {"content": [{"type": "text", "text": f"network unavailable: {exc.__class__.__name__}"}]}
    raise ValueError(f"unknown tool: {name}")


for line in sys.stdin:
    if not line.strip():
        continue
    try:
        request = json.loads(line)
        method = request.get("method")
        request_id = request.get("id")
        if method == "initialize":
            respond(request_id, {"protocolVersion": "2024-11-05", "capabilities": {"tools": {}}, "serverInfo": {"name": "kerna-filesystem-fixture", "version": "1.0.0"}})
        elif method == "notifications/initialized":
            continue
        elif method == "tools/list":
            respond(request_id, {"tools": tools()})
        elif method == "tools/call":
            params = request.get("params", {})
            respond(request_id, call(params.get("name", ""), params.get("arguments", {})))
        else:
            respond(request_id, error={"code": -32601, "message": f"unsupported method: {method}"})
    except Exception as exc:  # Fixture errors must remain protocol-visible.
        respond(request.get("id") if "request" in locals() else None, error={"code": -32000, "message": str(exc)})
