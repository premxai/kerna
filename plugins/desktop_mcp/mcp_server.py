#!/usr/bin/env python3
"""Kerna plugin: desktop automation (click, type, list dir).

Speaks standard MCP over stdio. Requires `pyautogui` only when a desktop tool
is actually called (lazy import), so the server still loads and lists its tools
on machines without it. Desktop control is high-risk — grant with approval.
"""
import sys
import json
import os

TOOLS = [
    {"name": "desktop_click", "description": "Click at screen coordinates (x, y).",
     "inputSchema": {"type": "object", "properties": {"x": {"type": "integer"}, "y": {"type": "integer"}}, "required": ["x", "y"]}},
    {"name": "desktop_type", "description": "Type text on the keyboard.",
     "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]}},
    {"name": "fs_list_dir", "description": "List files in a directory.",
     "inputSchema": {"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]}},
]


def call(name, args):
    if name == "fs_list_dir":
        return "\n".join(os.listdir(args["path"]))
    if name in ("desktop_click", "desktop_type"):
        import pyautogui  # lazy — only needed for real desktop control
        pyautogui.FAILSAFE = False
        if name == "desktop_click":
            pyautogui.click(x=args.get("x"), y=args.get("y"))
            return "clicked at (%s, %s)" % (args.get("x"), args.get("y"))
        pyautogui.write(args.get("text", ""), interval=0.01)
        return "typed: %s" % args.get("text", "")
    raise ValueError("unknown tool: %s" % name)


def handle(req):
    method = req.get("method")
    rid = req.get("id")
    if method == "initialize":
        return {"jsonrpc": "2.0", "id": rid, "result": {
            "protocolVersion": "2024-11-05", "capabilities": {"tools": {}},
            "serverInfo": {"name": "desktop", "version": "1.0.0"}}}
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
