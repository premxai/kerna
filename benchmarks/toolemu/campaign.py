#!/usr/bin/env python3
"""Pre-register and execute bounded ToolEmu permissive/governed pilot arms."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from datetime import UTC, datetime
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
RUNNER = Path(__file__).with_name("run_gateway.py")
DEFAULT_CAMPAIGN = Path(__file__).with_name("campaigns") / "todoist-deletion-replication-pilot.json"


def command_for(campaign: dict[str, Any], arm: str, trial: int, output: Path) -> list[str]:
    budget = campaign["budgets"]
    command = [
        sys.executable, str(RUNNER), "--execute", "--case", campaign["case"], "--arm", arm,
        "--agent-model", campaign["agentModel"], "--simulator-model", campaign["simulatorModel"],
        "--max-runtime-seconds", str(budget["maxRuntimeSeconds"]), "--max-llm-calls", str(budget["maxLlmCalls"]),
        "--max-tool-calls", str(budget["maxToolCalls"]), "--max-simulator-calls", str(budget["maxSimulatorCalls"]),
        "--max-cost-usd", str(budget["maxAgentCostUsd"]), "--output", str(output),
    ]
    if arm == "governed":
        for tool in campaign["governedAllowedTools"]:
            command.extend(["--allow-tool", tool])
    return command


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--campaign", type=Path, default=DEFAULT_CAMPAIGN)
    parser.add_argument("--trials", type=int)
    parser.add_argument("--output", type=Path, default=ROOT / "reports" / "toolemu-campaigns")
    group = parser.add_mutually_exclusive_group()
    group.add_argument("--execute-permissive", action="store_true")
    group.add_argument("--execute-governed", action="store_true")
    args = parser.parse_args()
    campaign = json.loads(args.campaign.read_text(encoding="utf-8"))
    trials = args.trials or campaign["trials"]
    if trials < 1:
        raise RuntimeError("--trials must be positive")
    arm = "permissive" if args.execute_permissive else "governed" if args.execute_governed else None
    timestamp = datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")
    output = args.output.resolve() / f"{campaign['name']}-{timestamp}"
    planned = [
        {"trial": trial, "permissive": command_for(campaign, "permissive", trial, output / f"permissive-trial-{trial:02d}"), "governed": command_for(campaign, "governed", trial, output / f"governed-trial-{trial:02d}")}
        for trial in range(1, trials + 1)
    ]
    plan = {"campaign": campaign, "trials": trials, "planned": planned, "executedAt": datetime.now(UTC).isoformat()}
    if arm is None:
        print(json.dumps({"dryRun": True, "plan": plan}, indent=2))
        return 0
    if not os.environ.get("OPENAI_API_KEY"):
        raise RuntimeError("OPENAI_API_KEY must be set before executing a ToolEmu campaign.")
    results: list[dict[str, Any]] = []
    for item in planned:
        command = item[arm]
        completed = subprocess.run(command, cwd=ROOT, text=True, encoding="utf-8", errors="replace", capture_output=True)
        try:
            payload = json.loads(completed.stdout)
        except json.JSONDecodeError:
            payload = None
        results.append({"trial": item["trial"], "returnCode": completed.returncode, "result": payload, "stderr": completed.stderr[-4000:]})
    output.mkdir(parents=True, exist_ok=True)
    result_path = output / f"{arm}-results.json"
    result_path.write_text(json.dumps({"campaign": campaign, "arm": arm, "results": results}, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"completed": True, "arm": arm, "trialsRun": len(results), "results": str(result_path)}, indent=2))
    return 0 if all(result["returnCode"] == 0 for result in results) else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except RuntimeError as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(2)
