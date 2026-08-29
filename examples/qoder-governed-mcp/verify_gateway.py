#!/usr/bin/env python3
"""Exercise the Kerna stdio MCP gateway without an IDE.

Run this from a directory containing a contract-generated ``kerna.toml``.
Set KERNA_BIN when Kerna is not already on PATH. The child MockMCP inherits the
same PATH, so a source-built Kerna executable is used for both processes.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path


def resolve_kerna() -> str:
    configured = os.environ.get("KERNA_BIN")
    if configured:
        path = Path(configured)
        if path.is_file():
            return str(path)
        raise SystemExit(f"KERNA_BIN does not point to a file: {path}")
    discovered = shutil.which("kerna")
    if discovered:
        return discovered
    raise SystemExit("Kerna was not found. Set KERNA_BIN or add kerna to PATH.")


def call(process: subprocess.Popen[str], request: dict) -> dict:
    assert process.stdin is not None and process.stdout is not None
    process.stdin.write(json.dumps(request) + "\n")
    process.stdin.flush()
    line = process.stdout.readline()
    if not line:
        stderr = process.stderr.read() if process.stderr else ""
        raise RuntimeError(f"gateway closed its stdout early: {stderr}")
    return json.loads(line)


def main() -> int:
    if not Path("kerna.toml").is_file():
        raise SystemExit("Run this from a contract workspace containing kerna.toml.")

    kerna = resolve_kerna()
    environment = os.environ.copy()
    # The generated downstream MockMCP command is `kerna mockmcp`; ensure it
    # resolves to the exact executable under test rather than an older install.
    environment["PATH"] = str(Path(kerna).parent) + os.pathsep + environment.get("PATH", "")
    process = subprocess.Popen(
        [kerna, "gateway"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        cwd=Path.cwd(),
        env=environment,
    )
    try:
        initialize = call(process, {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}})
        tools = call(process, {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}})
        echoed = call(process, {"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "echo", "arguments": {"text": "hello from Qoder"}}})
        blocked = call(process, {"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "network_probe", "arguments": {}}})
    finally:
        if process.stdin:
            process.stdin.close()
        process.wait(timeout=15)

    assert initialize["result"]["serverInfo"]["name"] == "kerna-gateway"
    assert any(tool["name"] == "echo" for tool in tools["result"]["tools"])
    assert echoed["result"]["content"][0]["text"] == "hello from Qoder"
    assert blocked["result"]["isError"] is True
    assert "denied by Kerna policy" in blocked["result"]["content"][0]["text"]
    print("PASS: echo was forwarded; network_probe was blocked by Kerna policy.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
