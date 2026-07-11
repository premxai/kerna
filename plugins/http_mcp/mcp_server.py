#!/usr/bin/env python3
"""Kerna starter plugin: generic HTTP/JSON API caller.

Speaks standard MCP over stdio. Pure Python standard library. Makes http(s)
GET/POST requests, bounded in size + time. Optional KERNA_HTTP_ALLOWLIST (comma-
separated hostnames) restricts which hosts may be called; if unset, any host is
allowed but every call is still governed by Kerna's policy + approval + budgets.
"""
import os
import sys
import json
import urllib.request
import urllib.parse

TIMEOUT = 20
MAX_BYTES = 200000


def _check_host(url):
    allow = os.environ.get("KERNA_HTTP_ALLOWLIST", "").strip()
    if not allow:
        return
    host = urllib.parse.urlparse(url).hostname or ""
    allowed = [h.strip().lower() for h in allow.split(",") if h.strip()]
    if host.lower() not in allowed:
        raise ValueError("host '%s' not in KERNA_HTTP_ALLOWLIST" % host)


def _request(method, url, headers=None, body=None):
    if not (url.startswith("http://") or url.startswith("https://")):
        raise ValueError("only http/https URLs are allowed")
    _check_host(url)
    data = body.encode("utf-8") if isinstance(body, str) else body
    req = urllib.request.Request(url, data=data, method=method,
                                 headers=headers or {"User-Agent": "kerna-http-mcp/1.0"})
    with urllib.request.urlopen(req, timeout=TIMEOUT) as resp:  # noqa: S310 — scheme+host checked
        return "HTTP %d\n%s" % (resp.status, resp.read(MAX_BYTES).decode("utf-8", errors="replace"))


TOOLS = [
    {"name": "http_get", "description": "HTTP GET a URL and return status + body.",
     "inputSchema": {"type": "object", "properties": {"url": {"type": "string"}}, "required": ["url"]}},
    {"name": "http_post_json", "description": "HTTP POST a JSON body to a URL.",
     "inputSchema": {"type": "object", "properties": {"url": {"type": "string"}, "json": {"type": "object"}}, "required": ["url", "json"]}},
]


def call(name, args):
    if name == "http_get":
        return _request("GET", args["url"])
    if name == "http_post_json":
        return _request("POST", args["url"],
                        headers={"Content-Type": "application/json", "User-Agent": "kerna-http-mcp/1.0"},
                        body=json.dumps(args.get("json", {})))
    raise ValueError("unknown tool: %s" % name)


def handle(req):
    method = req.get("method")
    rid = req.get("id")
    if method == "initialize":
        return {"jsonrpc": "2.0", "id": rid, "result": {
            "protocolVersion": "2024-11-05", "capabilities": {"tools": {}},
            "serverInfo": {"name": "http", "version": "1.0.0"}}}
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
