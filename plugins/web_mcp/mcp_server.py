#!/usr/bin/env python3
"""Kerna starter plugin: fetch a web page and extract readable text.

Speaks standard MCP over stdio. Pure Python standard library. Fetches are
bounded (size + timeout) and only http/https are allowed. Returns plain text
with HTML stripped, so an agent can read a page without a browser.
"""
import sys
import json
import urllib.request
from html.parser import HTMLParser

MAX_BYTES = 200000  # 200 KB cap on downloaded body
TIMEOUT = 15


class _TextExtractor(HTMLParser):
    def __init__(self):
        super().__init__()
        self.parts = []
        self._skip = 0

    def handle_starttag(self, tag, attrs):
        if tag in ("script", "style", "noscript"):
            self._skip += 1

    def handle_endtag(self, tag):
        if tag in ("script", "style", "noscript") and self._skip:
            self._skip -= 1

    def handle_data(self, data):
        if not self._skip:
            t = data.strip()
            if t:
                self.parts.append(t)


TOOLS = [
    {
        "name": "fetch_url",
        "description": "Fetch an http(s) URL and return the raw response body (bounded to 200 KB).",
        "inputSchema": {
            "type": "object",
            "properties": {"url": {"type": "string"}},
            "required": ["url"],
        },
    },
    {
        "name": "read_page_text",
        "description": "Fetch an http(s) page and return its readable text with HTML/scripts stripped.",
        "inputSchema": {
            "type": "object",
            "properties": {"url": {"type": "string"}},
            "required": ["url"],
        },
    },
]


def _fetch(url):
    if not (url.startswith("http://") or url.startswith("https://")):
        raise ValueError("only http/https URLs are allowed")
    req = urllib.request.Request(url, headers={"User-Agent": "kerna-web-mcp/1.0"})
    with urllib.request.urlopen(req, timeout=TIMEOUT) as resp:  # noqa: S310 — scheme checked above
        return resp.read(MAX_BYTES).decode("utf-8", errors="replace")


def call(name, args):
    if name == "fetch_url":
        return _fetch(args["url"])
    if name == "read_page_text":
        html = _fetch(args["url"])
        parser = _TextExtractor()
        parser.feed(html)
        text = "\n".join(parser.parts)
        return text[:MAX_BYTES]
    raise ValueError("unknown tool: %s" % name)


def handle(req):
    method = req.get("method")
    rid = req.get("id")
    if method == "initialize":
        return {"jsonrpc": "2.0", "id": rid, "result": {
            "protocolVersion": "2024-11-05",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "web", "version": "1.0.0"},
        }}
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
            return {"jsonrpc": "2.0", "id": rid, "result": {
                "isError": True, "content": [{"type": "text", "text": "error: %s" % e}]}}
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
