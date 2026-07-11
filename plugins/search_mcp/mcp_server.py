#!/usr/bin/env python3
"""Kerna starter plugin: web search.

Speaks standard MCP over stdio. Pure Python standard library. Uses the Tavily
search API (a simple, agent-friendly search endpoint); set TAVILY_API_KEY.
Kerna injects that secret into this plugin's environment — see
`kerna secrets add search`. Get a free key at https://tavily.com.
"""
import os
import sys
import json
import urllib.request

TIMEOUT = 20
MAX_RESULTS = 5


def web_search(query, max_results=MAX_RESULTS):
    key = os.environ.get("TAVILY_API_KEY", "").strip()
    if not key:
        raise ValueError(
            "TAVILY_API_KEY is not set. Run: kerna secrets add search "
            "(get a free key at https://tavily.com)"
        )
    body = json.dumps({
        "api_key": key,
        "query": query,
        "max_results": max_results,
        "include_answer": True,
    }).encode("utf-8")
    req = urllib.request.Request(
        "https://api.tavily.com/search",
        data=body,
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=TIMEOUT) as resp:  # noqa: S310 — fixed https host
        data = json.loads(resp.read().decode("utf-8"))
    lines = []
    if data.get("answer"):
        lines.append("Answer: " + data["answer"])
        lines.append("")
    for i, r in enumerate(data.get("results", [])[:max_results], 1):
        lines.append("%d. %s" % (i, r.get("title", "")))
        lines.append("   %s" % r.get("url", ""))
        snippet = (r.get("content", "") or "").strip().replace("\n", " ")
        if snippet:
            lines.append("   %s" % snippet[:200])
    return "\n".join(lines) if lines else "no results"


TOOLS = [
    {
        "name": "web_search",
        "description": "Search the web and return the top results with a short answer.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": {"type": "string"},
                "max_results": {"type": "integer", "default": 5},
            },
            "required": ["query"],
        },
    },
]


def call(name, args):
    if name == "web_search":
        return web_search(args["query"], int(args.get("max_results", MAX_RESULTS)))
    raise ValueError("unknown tool: %s" % name)


def handle(req):
    method = req.get("method")
    rid = req.get("id")
    if method == "initialize":
        return {"jsonrpc": "2.0", "id": rid, "result": {
            "protocolVersion": "2024-11-05", "capabilities": {"tools": {}},
            "serverInfo": {"name": "search", "version": "1.0.0"}}}
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
