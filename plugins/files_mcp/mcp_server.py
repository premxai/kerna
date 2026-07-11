#!/usr/bin/env python3
"""Kerna starter plugin: safe workspace file operations.

Speaks standard MCP over stdio (initialize / tools/list / tools/call). Pure
Python standard library — no pip install required. All paths are confined to
the working directory Kerna spawns the plugin in (the sandbox), so it cannot
read or write outside the workspace.
"""
import sys
import json
import os

ROOT = os.path.abspath(os.getcwd())
MAX_BYTES = 50000


def _safe(path):
    """Resolve path inside ROOT; raise if it escapes the workspace."""
    target = os.path.abspath(os.path.join(ROOT, path or "."))
    if target != ROOT and not target.startswith(ROOT + os.sep):
        raise ValueError("path escapes the workspace boundary")
    return target


TOOLS = [
    {
        "name": "read_file",
        "description": "Read a UTF-8 text file inside the workspace.",
        "inputSchema": {
            "type": "object",
            "properties": {"path": {"type": "string"}},
            "required": ["path"],
        },
    },
    {
        "name": "write_file",
        "description": "Write (create/overwrite) a text file inside the workspace.",
        "inputSchema": {
            "type": "object",
            "properties": {"path": {"type": "string"}, "content": {"type": "string"}},
            "required": ["path", "content"],
        },
    },
    {
        "name": "list_dir",
        "description": "List files and folders in a workspace directory (default: root).",
        "inputSchema": {
            "type": "object",
            "properties": {"path": {"type": "string"}},
        },
    },
    {
        "name": "search_text",
        "description": "Search workspace files for a substring (grep-like). Returns path:line matches.",
        "inputSchema": {
            "type": "object",
            "properties": {"query": {"type": "string"}, "path": {"type": "string"}},
            "required": ["query"],
        },
    },
]


def call(name, args):
    if name == "read_file":
        with open(_safe(args["path"]), "r", encoding="utf-8", errors="replace") as f:
            data = f.read(MAX_BYTES + 1)
        if len(data) > MAX_BYTES:
            data = data[:MAX_BYTES] + "\n[... truncated at 50 KB]"
        return data
    if name == "write_file":
        target = _safe(args["path"])
        os.makedirs(os.path.dirname(target) or ".", exist_ok=True)
        with open(target, "w", encoding="utf-8") as f:
            f.write(args.get("content", ""))
        return "wrote %d bytes to %s" % (len(args.get("content", "")), args["path"])
    if name == "list_dir":
        target = _safe(args.get("path", "."))
        entries = []
        for e in sorted(os.listdir(target)):
            full = os.path.join(target, e)
            entries.append(("%s/" % e) if os.path.isdir(full) else e)
        return "\n".join(entries) if entries else "(empty)"
    if name == "search_text":
        query = args["query"]
        base = _safe(args.get("path", "."))
        hits = []
        for dirpath, _dirs, files in os.walk(base):
            for fn in files:
                fp = os.path.join(dirpath, fn)
                try:
                    with open(fp, "r", encoding="utf-8", errors="ignore") as f:
                        for i, line in enumerate(f, 1):
                            if query in line:
                                rel = os.path.relpath(fp, ROOT)
                                hits.append("%s:%d: %s" % (rel, i, line.strip()[:120]))
                                if len(hits) >= 100:
                                    return "\n".join(hits) + "\n[... 100-match cap]"
                except (OSError, UnicodeDecodeError):
                    continue
        return "\n".join(hits) if hits else "no matches"
    raise ValueError("unknown tool: %s" % name)


def handle(req):
    method = req.get("method")
    rid = req.get("id")
    if method == "initialize":
        return {"jsonrpc": "2.0", "id": rid, "result": {
            "protocolVersion": "2024-11-05",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "files", "version": "1.0.0"},
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
        except Exception as e:  # noqa: BLE001 — report tool errors as isError
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
