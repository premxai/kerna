#!/usr/bin/env python3
"""Check whether an AgentDojo/Kerna evaluation environment is ready.

This script intentionally performs no benchmark run and makes no model request.
AgentDojo's stock CLI executes its tools directly, so it is not a Kerna-governed
evaluation. A run is publishable only after the bridge described in this
directory's README records Kerna receipts for every tool action.
"""

from __future__ import annotations

import importlib.metadata
import json
import shutil
import sys
from pathlib import Path


REQUIRED_AGENTDOJO_VERSION = "0.1.35"
REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
BRIDGE_PATH = Path(__file__).with_name("kerna_agentdojo_mcp.py")


def package_version(name: str) -> str | None:
    try:
        return importlib.metadata.version(name)
    except importlib.metadata.PackageNotFoundError:
        return None


def main() -> int:
    agentdojo_version = package_version("agentdojo")
    kerna_executable = shutil.which("kerna")
    local_binary = REPOSITORY_ROOT / "target" / "debug" / (
        "kerna.exe" if sys.platform == "win32" else "kerna"
    )
    if kerna_executable is None and local_binary.is_file():
        kerna_executable = str(local_binary)

    checks = {
        "python": {
            "required": ">=3.10",
            "actual": ".".join(map(str, sys.version_info[:3])),
            "passed": sys.version_info >= (3, 10),
        },
        "agentdojo": {
            "required": REQUIRED_AGENTDOJO_VERSION,
            "actual": agentdojo_version,
            "passed": agentdojo_version == REQUIRED_AGENTDOJO_VERSION,
        },
        "kerna_binary": {
            "required": "kerna on PATH or a local debug build",
            "actual": kerna_executable,
            "passed": kerna_executable is not None,
        },
        "governed_bridge": {
            "required": "an AgentDojo FunctionsRuntime-to-MCP bridge with receipt export",
            "actual": str(BRIDGE_PATH) if BRIDGE_PATH.is_file() else "missing",
            "passed": BRIDGE_PATH.is_file(),
        },
    }
    ready = all(check["passed"] for check in checks.values())
    report = {
        "benchmark": "AgentDojo / Kerna integration preflight",
        "adapterStatus": "bridge-ready, no external result published",
        "publishableResult": False,
        "ready": ready,
        "checks": checks,
        "nextCommand": "python -m pip install -r benchmarks/agentdojo/requirements.txt",
    }
    print(json.dumps(report, indent=2))
    return 0 if ready else 1


if __name__ == "__main__":
    raise SystemExit(main())
