#!/usr/bin/env python3
"""No-cost readiness check for the pinned BFCL provider-compatibility pilot."""

from __future__ import annotations

import argparse
import importlib.metadata
import json
import os
import shutil
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
EXPECTED_PACKAGE = "bfcl-eval"
EXPECTED_VERSION = "2025.12.17"
PILOT_IDS = Path(__file__).with_name("pilot-ids.json")


def bfcl_command() -> str | None:
    executable = "bfcl.exe" if os.name == "nt" else "bfcl"
    sibling = Path(sys.executable).with_name(executable)
    if sibling.exists():
        return str(sibling)
    return shutil.which("bfcl")


def cli_check(command: str | None) -> dict[str, object]:
    if command is None:
        return {"available": False, "detail": "bfcl executable was not found"}
    try:
        completed = subprocess.run(
            [command, "--help"],
            capture_output=True,
            text=True,
            timeout=30,
            check=False,
        )
    except OSError as error:
        return {"available": False, "detail": str(error)}
    except subprocess.TimeoutExpired:
        return {"available": False, "detail": "bfcl --help timed out"}
    detail = (completed.stderr or completed.stdout).strip()
    return {
        "available": completed.returncode == 0,
        "exitCode": completed.returncode,
        "detail": detail[-500:] if detail else "bfcl --help completed",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--require-provider",
        action="store_true",
        help="fail when OPENAI_API_KEY is absent; the default check makes no provider calls",
    )
    parser.add_argument(
        "--out",
        default="reports/bfcl/preflight.json",
        help="JSON report path relative to the repository root",
    )
    args = parser.parse_args()

    try:
        installed_version = importlib.metadata.version(EXPECTED_PACKAGE)
    except importlib.metadata.PackageNotFoundError:
        installed_version = None

    pilot = json.loads(PILOT_IDS.read_text(encoding="utf-8"))
    case_ids = pilot.get("simple_python", [])
    ids_are_valid = (
        isinstance(case_ids, list)
        and len(case_ids) == 10
        and all(isinstance(case_id, str) and case_id.startswith("simple_python_") for case_id in case_ids)
    )
    command = bfcl_command()
    cli = cli_check(command)
    checks = {
        "packageInstalled": installed_version is not None,
        "packageVersionPinned": installed_version == EXPECTED_VERSION,
        "cliOperational": cli["available"],
        "pilotFixtureValid": ids_are_valid,
        "providerCredentialPresent": bool(os.environ.get("OPENAI_API_KEY")),
    }
    required_checks = [
        checks["packageInstalled"],
        checks["packageVersionPinned"],
        checks["cliOperational"],
        checks["pilotFixtureValid"],
    ]
    if args.require_provider:
        required_checks.append(checks["providerCredentialPresent"])

    report = {
        "benchmark": "BFCL provider compatibility pilot preflight",
        "version": 1,
        "executedAt": datetime.now(timezone.utc).isoformat(),
        "classification": "Provider/model native function-calling compatibility only. This is not a Kerna safety, utility, or policy-enforcement score.",
        "configuration": {
            "package": EXPECTED_PACKAGE,
            "requiredVersion": EXPECTED_VERSION,
            "pilotCategory": "simple_python",
            "pilotCaseCount": len(case_ids),
            "requiresProviderCredential": args.require_provider,
        },
        "checks": checks,
        "installedVersion": installed_version,
        "bfclCommand": command,
        "bfclCli": cli,
        "readyForExecution": all(required_checks),
    }
    output = (REPO_ROOT / args.out).resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))
    return 0 if report["readyForExecution"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
