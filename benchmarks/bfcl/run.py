#!/usr/bin/env python3
"""Run a bounded, non-live BFCL provider-compatibility pilot."""

from __future__ import annotations

import argparse
import hashlib
import importlib.metadata
import json
import os
import shutil
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
EXPECTED_PACKAGE = "bfcl-eval"
EXPECTED_VERSION = "2025.12.17"
PILOT_IDS = Path(__file__).with_name("pilot-ids.json")
DEFAULT_MODEL = "gpt-4.1-nano-2025-04-14-FC"


def bfcl_command() -> str | None:
    executable = "bfcl.exe" if os.name == "nt" else "bfcl"
    sibling = Path(sys.executable).with_name(executable)
    if sibling.exists():
        return str(sibling)
    return shutil.which("bfcl")


def command_result(command: list[str], environment: dict[str, str], timeout_seconds: int) -> None:
    try:
        completed = subprocess.run(
            command,
            cwd=REPO_ROOT,
            env=environment,
            timeout=timeout_seconds,
            check=False,
        )
    except subprocess.TimeoutExpired as error:
        raise RuntimeError("BFCL command timed out after " + str(timeout_seconds) + " seconds") from error
    if completed.returncode != 0:
        raise RuntimeError("BFCL command failed with exit code " + str(completed.returncode))


def score_files(root: Path) -> list[dict[str, str]]:
    files = []
    for path in sorted((root / "score").rglob("*.json")) if (root / "score").exists() else []:
        files.append(
            {
                "path": path.relative_to(root).as_posix(),
                "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
            }
        )
    return files


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", default=DEFAULT_MODEL, help="BFCL-supported function-calling model ID")
    parser.add_argument("--num-threads", type=int, default=1, help="API inference concurrency; pilot requires 1")
    parser.add_argument(
        "--command-timeout-seconds",
        type=int,
        default=600,
        help="maximum duration for each official BFCL generate or evaluate command",
    )
    parser.add_argument("--execute", action="store_true", help="make provider calls after all guard checks pass")
    parser.add_argument(
        "--out",
        default="reports/bfcl/latest.json",
        help="redacted aggregate path relative to the repository root",
    )
    args = parser.parse_args()
    if args.num_threads != 1:
        raise SystemExit("The bounded pilot requires --num-threads 1")
    if not 1 <= args.command_timeout_seconds <= 3600:
        raise SystemExit("--command-timeout-seconds must be 1..3600")

    try:
        installed_version = importlib.metadata.version(EXPECTED_PACKAGE)
    except importlib.metadata.PackageNotFoundError as error:
        raise SystemExit("Install the pinned BFCL package before running: " + str(error))
    if installed_version != EXPECTED_VERSION:
        raise SystemExit("Expected " + EXPECTED_PACKAGE + "==" + EXPECTED_VERSION + ", found " + installed_version)

    command = bfcl_command()
    if command is None:
        raise SystemExit("The bfcl executable was not found in this environment")
    cli = subprocess.run([command, "--help"], capture_output=True, text=True, check=False)
    if cli.returncode != 0:
        detail = (cli.stderr or cli.stdout).strip()
        raise SystemExit("The bfcl CLI is not operational; run preflight after a complete install. " + detail[-300:])
    pilot = json.loads(PILOT_IDS.read_text(encoding="utf-8"))
    case_ids = pilot.get("simple_python", [])
    if not isinstance(case_ids, list) or len(case_ids) != 10:
        raise SystemExit("pilot-ids.json must contain exactly 10 simple_python case IDs")

    planned = {
        "benchmark": "BFCL provider compatibility pilot",
        "version": 1,
        "executedAt": datetime.now(timezone.utc).isoformat(),
        "classification": "Provider/model native function-calling compatibility only. Do not treat this as a Kerna safety, utility, or policy-enforcement score.",
        "configuration": {
            "bfclPackage": EXPECTED_PACKAGE,
            "bfclVersion": EXPECTED_VERSION,
            "model": args.model,
            "testCategory": "simple_python",
            "caseIds": case_ids,
            "numThreads": args.num_threads,
            "commandTimeoutSeconds": args.command_timeout_seconds,
            "liveData": False,
            "providerCallsAuthorized": args.execute,
        },
    }
    output = (REPO_ROOT / args.out).resolve()
    output.parent.mkdir(parents=True, exist_ok=True)

    if not args.execute:
        planned["status"] = "planned"
        planned["nextCommand"] = "Run this script with --execute from a terminal that already has OPENAI_API_KEY set."
        output.write_text(json.dumps(planned, indent=2) + "\n", encoding="utf-8")
        print(json.dumps(planned, indent=2))
        return 0
    if not os.environ.get("OPENAI_API_KEY"):
        raise SystemExit("OPENAI_API_KEY is required in the current environment. Do not place it in a committed file.")

    run_root = output.parent / "runs" / datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    run_root.mkdir(parents=True, exist_ok=False)
    (run_root / "test_case_ids_to_generate.json").write_text(json.dumps(pilot, indent=2) + "\n", encoding="utf-8")
    environment = os.environ.copy()
    environment["BFCL_PROJECT_ROOT"] = str(run_root)
    started = time.monotonic()
    try:
        command_result(
            [command, "generate", "--model", args.model, "--run-ids", "--num-threads", "1"],
            environment,
            args.command_timeout_seconds,
        )
        command_result(
            [command, "evaluate", "--model", args.model, "--test-category", "simple_python", "--partial-eval"],
            environment,
            args.command_timeout_seconds,
        )
    except RuntimeError as error:
        planned["status"] = "failed"
        planned["durationSeconds"] = round(time.monotonic() - started, 3)
        planned["runRoot"] = str(run_root.relative_to(REPO_ROOT))
        planned["error"] = str(error)
        output.write_text(json.dumps(planned, indent=2) + "\n", encoding="utf-8")
        print(json.dumps(planned, indent=2))
        return 1

    planned["status"] = "completed"
    planned["durationSeconds"] = round(time.monotonic() - started, 3)
    planned["runRoot"] = str(run_root.relative_to(REPO_ROOT))
    planned["scoreFiles"] = score_files(run_root)
    output.write_text(json.dumps(planned, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(planned, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
