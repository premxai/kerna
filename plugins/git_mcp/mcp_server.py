#!/usr/bin/env python3
"""Kerna starter plugin: read-only git inspection for the workspace repo.

Speaks standard MCP over stdio. Pure Python standard library (shells out to the
`git` binary). Exposes only read-only porcelain — status, log, diff, show,
current-branch — never commands that mutate the repo or the network.
"""
import sys
import json
import subprocess

TIMEOUT = 20
MAX_BYTES = 50000

# Only these read-only git subcommands are permitted, with fixed safe flags.
COMMANDS = {
    "git_status": ["status", "--short", "--branch"],
    "git_branch": ["rev-parse", "--abbrev-ref", "HEAD"],
    "git_log": ["log", "--oneline", "-n", "20"],
    "git_diff": ["diff", "--stat"],
}

TOOLS = [
    {"name": "git_status", "description": "Show `git status` (short) for the workspace repo.",
     "inputSchema": {"type": "object", "properties": {}}},
    {"name": "git_branch", "description": "Show the current git branch.",
     "inputSchema": {"type": "object", "properties": {}}},
    {"name": "git_log", "description": "Show the last 20 commits (one line each).",
     "inputSchema": {"type": "object", "properties": {}}},
    {"name": "git_diff", "description": "Show a diffstat of uncommitted changes.",
     "inputSchema": {"type": "object", "properties": {}}},
]


def call(name, _args):
    argv = COMMANDS.get(name)
    if argv is None:
        raise ValueError("unknown tool: %s" % name)
    proc = subprocess.run(  # noqa: S603 — fixed arg lists, no shell
        ["git"] + argv,
        capture_output=True, text=True, timeout=TIMEOUT,
    )
    out = (proc.stdout or "") + (proc.stderr or "")
    if not out.strip():
        out = "(no output)"
    return out[:MAX_BYTES]


def handle(req):
    method = req.get("method")
    rid = req.get("id")
    if method == "initialize":
        return {"jsonrpc": "2.0", "id": rid, "result": {
            "protocolVersion": "2024-11-05",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "git", "version": "1.0.0"},
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
