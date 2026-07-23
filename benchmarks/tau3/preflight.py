#!/usr/bin/env python3
"""No-cost preflight for the pinned tau3 native-control pilot."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
from datetime import datetime, timezone
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
PINNED_TAU3_REVISION = "1d244f5dca42944b67a379b44bfeb9f5748f189d"


def run(command: list[str], cwd: Path, environment: dict[str, str]) -> dict[str, object]:
    try:
        completed = subprocess.run(command, cwd=cwd, env=environment, capture_output=True, text=True, timeout=120)
    except (OSError, subprocess.TimeoutExpired) as error:
        return {"ok": False, "detail": str(error)}
    detail = (completed.stdout + completed.stderr).strip()
    return {"ok": completed.returncode == 0, "exitCode": completed.returncode, "detail": detail[-1000:]}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-root", default="reports/tau3-source", help="tau3 source checkout relative to the repository")
    parser.add_argument("--require-provider", action="store_true", help="also require OPENAI_API_KEY without printing it")
    parser.add_argument("--out", default="reports/tau3/preflight.json", help="JSON report path relative to the repository")
    args = parser.parse_args()

    source_root = (REPO_ROOT / args.source_root).resolve()
    environment = os.environ.copy()
    environment["PYTHONUTF8"] = "1"
    uv = shutil.which("uv")
    source_exists = source_root.is_dir()
    revision = run(["git", "rev-parse", "HEAD"], source_root, environment) if source_exists else {"ok": False, "detail": "source checkout missing"}
    revision_value = revision.get("detail", "").strip().splitlines()[-1] if revision.get("ok") else None
    data = run([uv, "run", "tau2", "check-data"], source_root, environment) if uv and source_exists else {"ok": False, "detail": "uv or source checkout missing"}
    checks = {
        "uvAvailable": uv is not None,
        "sourceCheckoutPresent": source_exists,
        "sourceRevisionPinned": revision_value == PINNED_TAU3_REVISION,
        "dataReady": data["ok"],
        "providerCredentialPresent": bool(os.environ.get("OPENAI_API_KEY")),
    }
    required = [checks["uvAvailable"], checks["sourceCheckoutPresent"], checks["sourceRevisionPinned"], checks["dataReady"]]
    if args.require_provider:
        required.append(checks["providerCredentialPresent"])
    report = {
        "benchmark": "tau3 native-control pilot preflight",
        "version": 1,
        "executedAt": datetime.now(timezone.utc).isoformat(),
        "classification": "Native tau3 utility-control readiness only. It is not a Kerna result until the same tool calls traverse the Kerna gateway adapter.",
        "configuration": {
            "tau3Revision": PINNED_TAU3_REVISION,
            "domain": "retail",
            "taskIds": ["0", "1", "2"],
            "trials": 1,
            "model": "gpt-4o-mini",
            "maxConcurrency": 1,
            "maxSteps": 60,
            "timeoutSeconds": 300,
            "seed": 300,
            "requiresProviderCredential": args.require_provider,
        },
        "checks": checks,
        "observedRevision": revision_value,
        "dataCheck": data,
        "readyForNativeControl": all(required),
    }
    output = (REPO_ROOT / args.out).resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))
    return 0 if report["readyForNativeControl"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
