#!/usr/bin/env python3
"""Kerna starter plugin: personal markdown notes.

Speaks standard MCP over stdio. Pure Python standard library. Notes are plain
markdown files stored in a `notes/` folder inside the workspace — confined to
the sandbox, nothing leaves your machine.
"""
import os
import sys
import json
import datetime

ROOT = os.path.abspath(os.getcwd())
NOTES_DIR = os.path.join(ROOT, "notes")


def _path(title):
    safe = "".join(c if c.isalnum() or c in (" ", "-", "_") else "_" for c in title).strip()
    safe = safe.replace(" ", "-").lower() or "note"
    return os.path.join(NOTES_DIR, safe + ".md")


TOOLS = [
    {"name": "add_note", "description": "Create or append to a markdown note.",
     "inputSchema": {"type": "object", "properties": {"title": {"type": "string"}, "content": {"type": "string"}}, "required": ["title", "content"]}},
    {"name": "list_notes", "description": "List all notes.",
     "inputSchema": {"type": "object", "properties": {}}},
    {"name": "read_note", "description": "Read a note by title.",
     "inputSchema": {"type": "object", "properties": {"title": {"type": "string"}}, "required": ["title"]}},
    {"name": "search_notes", "description": "Search notes for a substring.",
     "inputSchema": {"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]}},
]


def call(name, args):
    os.makedirs(NOTES_DIR, exist_ok=True)
    if name == "add_note":
        p = _path(args["title"])
        stamp = datetime.datetime.now().strftime("%Y-%m-%d %H:%M")
        with open(p, "a", encoding="utf-8") as f:
            f.write("\n## %s\n%s\n" % (stamp, args.get("content", "")))
        return "saved to %s" % os.path.relpath(p, ROOT)
    if name == "list_notes":
        if not os.path.isdir(NOTES_DIR):
            return "(no notes yet)"
        files = sorted(f for f in os.listdir(NOTES_DIR) if f.endswith(".md"))
        return "\n".join(files) if files else "(no notes yet)"
    if name == "read_note":
        p = _path(args["title"])
        if not os.path.exists(p):
            return "note not found: %s" % args["title"]
        with open(p, "r", encoding="utf-8") as f:
            return f.read()[:50000]
    if name == "search_notes":
        q = args["query"]
        hits = []
        if os.path.isdir(NOTES_DIR):
            for fn in sorted(os.listdir(NOTES_DIR)):
                if not fn.endswith(".md"):
                    continue
                with open(os.path.join(NOTES_DIR, fn), "r", encoding="utf-8", errors="ignore") as f:
                    for i, line in enumerate(f, 1):
                        if q.lower() in line.lower():
                            hits.append("%s:%d: %s" % (fn, i, line.strip()[:120]))
        return "\n".join(hits) if hits else "no matches"
    raise ValueError("unknown tool: %s" % name)


def handle(req):
    method = req.get("method")
    rid = req.get("id")
    if method == "initialize":
        return {"jsonrpc": "2.0", "id": rid, "result": {
            "protocolVersion": "2024-11-05", "capabilities": {"tools": {}},
            "serverInfo": {"name": "notes", "version": "1.0.0"}}}
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
