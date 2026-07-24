#!/usr/bin/env python3
"""Minimal stdio MCP bridge from Kerna to one local tau3 environment.

The benchmark parent owns the tau3 environment and exposes it only on loopback.
This process deliberately contains no domain logic or environment state. Kerna
starts it as an untrusted MCP child and it forwards each MCP request to the
parent using a per-run bearer token.
"""

from __future__ import annotations

import argparse
import json
import sys
import urllib.error
import urllib.request
from typing import Any


def request(endpoint: str, token: str, payload: dict[str, Any]) -> dict[str, Any]:
    body = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(
        endpoint,
        data=body,
        headers={"Content-Type": "application/json", "Authorization": f"Bearer {token}"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=20) as response:
            return json.loads(response.read().decode("utf-8"))
    except (urllib.error.URLError, urllib.error.HTTPError, json.JSONDecodeError) as error:
        raise RuntimeError(f"loopback tau3 bridge request failed: {error}") from error


def reply(request_id: Any, result: dict[str, Any]) -> None:
    print(json.dumps({"jsonrpc": "2.0", "id": request_id, "result": result}), flush=True)


def error(request_id: Any, message: str) -> None:
    print(
        json.dumps({"jsonrpc": "2.0", "id": request_id, "error": {"code": -32603, "message": message}}),
        flush=True,
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--endpoint", required=True, help="loopback parent endpoint, e.g. http://127.0.0.1:1234")
    parser.add_argument("--token", required=True, help="per-run loopback bearer token")
    args = parser.parse_args()

    for line in sys.stdin:
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
                        "serverInfo": {"name": "kerna-tau3-loopback-bridge", "version": "0.1.0"},
                    },
                )
            elif method == "notifications/initialized":
                continue
            elif method == "tools/list":
                reply(request_id, {"tools": request(args.endpoint + "/tools", args.token, {})["tools"]})
            elif method == "tools/call":
                params = message.get("params") or {}
                upstream = request(
                    args.endpoint + "/call",
                    args.token,
                    {"name": params.get("name"), "arguments": params.get("arguments") or {}},
                )
                text = json.dumps(upstream["toolMessage"], separators=(",", ":"))
                reply(
                    request_id,
                    {"content": [{"type": "text", "text": text}], "isError": bool(upstream["toolMessage"].get("error"))},
                )
            elif request_id is not None:
                error(request_id, f"Method not found: {method}")
        except Exception as exc:  # Return a protocol error; never print logs to MCP stdout.
            error(locals().get("request_id"), str(exc))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
