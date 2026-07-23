#!/usr/bin/env python3
"""Run the pre-registered tau3 native utility control, never a Kerna arm."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
from datetime import datetime, timezone
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
TAU_ROOT = REPO_ROOT / "reports" / "tau3-source"
TASK_IDS = ["0", "1", "2"]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--execute", action="store_true", help="make the bounded native-control provider calls")
    parser.add_argument("--model", default="gpt-4.1-nano", help="same LiteLLM model for agent and user simulator")
    parser.add_argument("--out", default="reports/tau3/native-control-plan.json", help="wrapper JSON report path")
    args = parser.parse_args()
    if not TAU_ROOT.is_dir():
        raise SystemExit("Pinned tau3 checkout is missing. Run preflight first.")
    uv = shutil.which("uv")
    if uv is None:
        raise SystemExit("uv is required for the pinned tau3 checkout")

    run_name = "kerna-native-retail-pilot-" + datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    report = {
        "benchmark": "tau3 native retail utility control",
        "version": 1,
        "executedAt": datetime.now(timezone.utc).isoformat(),
        "classification": "Native tau3 control only. This does not invoke Kerna and cannot support a Kerna utility or safety claim.",
        "configuration": {
            "domain": "retail",
            "taskIds": TASK_IDS,
            "trials": 1,
            "agent": "llm_agent",
            "user": "user_simulator",
            "agentModel": args.model,
            "userModel": args.model,
            "maxConcurrency": 1,
            "maxSteps": 20,
            "maxErrors": 5,
            "timeoutSeconds": 300,
            "seed": 300,
        },
    }
    output = (REPO_ROOT / args.out).resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    if not args.execute:
        report["status"] = "planned"
        report["nextCommand"] = "Re-run with --execute from a terminal that already has OPENAI_API_KEY set."
        output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        print(json.dumps(report, indent=2))
        return 0
    if not os.environ.get("OPENAI_API_KEY"):
        raise SystemExit("OPENAI_API_KEY is required in the executing terminal. Do not place it in a committed file.")

    environment = os.environ.copy()
    environment["PYTHONUTF8"] = "1"
    command = [
        uv, "run", "tau2", "run",
        "--domain", "retail",
        "--agent", "llm_agent",
        "--agent-llm", args.model,
        "--user", "user_simulator",
        "--user-llm", args.model,
        "--num-trials", "1",
        "--task-ids", *TASK_IDS,
        "--max-concurrency", "1",
        "--max-steps", "20",
        "--max-errors", "5",
        "--timeout", "300",
        "--seed", "300",
        "--save-to", run_name,
    ]
    completed = subprocess.run(command, cwd=TAU_ROOT, env=environment, check=False)
    report["status"] = "completed" if completed.returncode == 0 else "failed"
    report["returnCode"] = completed.returncode
    report["resultsPath"] = str((TAU_ROOT / "data" / "simulations" / run_name / "results.json").relative_to(REPO_ROOT))
    output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))
    return completed.returncode


if __name__ == "__main__":
    raise SystemExit(main())
