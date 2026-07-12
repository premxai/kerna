#!/usr/bin/env python3
"""Kerna starter plugin: weather.

Speaks standard MCP over stdio. Pure Python standard library. Uses wttr.in,
a free, no-key weather service. Read-only.
"""
import sys
import json
import urllib.request
import urllib.parse

TIMEOUT = 15


def get_weather(location):
    loc = urllib.parse.quote(location.strip() or "")
    url = "https://wttr.in/%s?format=j1" % loc
    req = urllib.request.Request(url, headers={"User-Agent": "kerna-weather"})
    with urllib.request.urlopen(req, timeout=TIMEOUT) as resp:  # noqa: S310 fixed https host
        data = json.loads(resp.read().decode("utf-8"))
    cur = (data.get("current_condition") or [{}])[0]
    desc = ((cur.get("weatherDesc") or [{}])[0]).get("value", "")
    lines = [
        "%s: %s, %s°C (feels %s°C), humidity %s%%, wind %s km/h"
        % (
            location,
            desc,
            cur.get("temp_C", "?"),
            cur.get("FeelsLikeC", "?"),
            cur.get("humidity", "?"),
            cur.get("windspeedKmph", "?"),
        )
    ]
    for day in (data.get("weather") or [])[:3]:
        d = day.get("date", "")
        lines.append(
            "  %s: %s–%s°C" % (d, day.get("mintempC", "?"), day.get("maxtempC", "?"))
        )
    return "\n".join(lines)


TOOLS = [
    {
        "name": "get_weather",
        "description": "Current weather and a 3-day outlook for a location (city name, airport code, etc.).",
        "inputSchema": {
            "type": "object",
            "properties": {"location": {"type": "string"}},
            "required": ["location"],
        },
    },
]


def call(name, args):
    if name == "get_weather":
        return get_weather(args["location"])
    raise ValueError("unknown tool: %s" % name)


def handle(req):
    method = req.get("method")
    rid = req.get("id")
    if method == "initialize":
        return {"jsonrpc": "2.0", "id": rid, "result": {
            "protocolVersion": "2024-11-05", "capabilities": {"tools": {}},
            "serverInfo": {"name": "weather", "version": "1.0.0"}}}
    if method in ("notifications/initialized", "notifications/cancelled"):
        return None
    if method == "tools/list":
        return {"jsonrpc": "2.0", "id": rid, "result": {"tools": TOOLS}}
    if method == "tools/call":
        params = req.get("params", {})
        try:
            text = call(params.get("name"), params.get("arguments", {}))
            return {"jsonrpc": "2.0", "id": rid, "result": {"content": [{"type": "text", "text": text}]}}
        except Exception as e:  # noqa: BLE001
            return {"jsonrpc": "2.0", "id": rid, "result": {"isError": True, "content": [{"type": "text", "text": "error: %s" % e}]}}
    return {"jsonrpc": "2.0", "id": rid, "error": {"code": -32601, "message": "method not found"}}


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            resp = handle(json.loads(line))
        except Exception:  # noqa: BLE001
            resp = {"jsonrpc": "2.0", "id": None, "error": {"code": -32700, "message": "parse error"}}
        if resp is not None:
            sys.stdout.write(json.dumps(resp) + "\n")
            sys.stdout.flush()


if __name__ == "__main__":
    main()
