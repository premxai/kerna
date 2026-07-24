#!/usr/bin/env python3
"""Thin stdio MCP child that forwards ToolEmu calls to its local parent."""

from __future__ import annotations

import argparse
import json
import sys
import urllib.error
import urllib.request
from typing import Any


def post(endpoint: str, token: str, payload: dict[str, Any]) -> dict[str, Any]:
    request = urllib.request.Request(
        endpoint,
        data=json.dumps(payload).encode("utf-8"),
        headers={"Content-Type": "application/json", "Authorization": f"Bearer {token}"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=20) as response:
            return json.loads(response.read().decode("utf-8"))
    except (urllib.error.URLError, urllib.error.HTTPError, json.JSONDecodeError) as error:
        raise RuntimeError(f"ToolEmu loopback request failed: {error}") from error


def reply(request_id: Any, result: dict[str, Any]) -> None:
    print(json.dumps({"jsonrpc": "2.0", "id": request_id, "result": result}), flush=True)


def fail(request_id: Any, message: str) -> None:
    print(
        json.dumps({"jsonrpc": "2.0", "id": request_id, "error": {"code": -32603, "message": message}}),
        flush=True,
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--endpoint", required=True)
    parser.add_argument("--token", required=True)
    args = parser.parse_args()

    for line in sys.stdin:
        request_id: Any = None
        try:
            message = json.loads(line)
            request_id = message.get("id")
            method = message.get("method")
            if method == "initialize":
                reply(
                    request_id,
                    {
                        "protocolVersion": "2025-06-18",
                        "capabilities": {"tools": {}},
                        "serverInfo": {"name": "kerna-toolemu-loopback-bridge", "version": "0.1.0"},
                    },
                )
            elif method == "notifications/initialized":
                continue
            elif method == "tools/list":
                reply(request_id, {"tools": post(args.endpoint + "/tools", args.token, {})["tools"]})
            elif method == "tools/call":
                params = message.get("params") or {}
                response = post(
                    args.endpoint + "/call",
                    args.token,
                    {"name": params.get("name"), "arguments": params.get("arguments") or {}},
                )
                observation = response["observation"]
                text = observation if isinstance(observation, str) else json.dumps(observation, separators=(",", ":"))
                reply(request_id, {"content": [{"type": "text", "text": text}], "isError": bool(response.get("isError"))})
            elif request_id is not None:
                fail(request_id, f"Method not found: {method}")
        except Exception as error:  # Keep all errors inside the JSON-RPC stream.
            fail(request_id, str(error))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
