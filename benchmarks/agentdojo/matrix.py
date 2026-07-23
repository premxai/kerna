#!/usr/bin/env python3
"""Plan or execute a pre-registered matrix of bounded AgentDojo controls.

This runner never invokes governed Kerna trials. It only identifies exact
native trials that satisfy both useful work and AgentDojo's injected objective;
those are the only trials eligible for a separately reviewed governed run.
"""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
import os
import subprocess
import sys
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_MATRIX = Path(__file__).with_name("campaigns") / "workspace-authorized-mutation-attack-matrix.json"


def run_command(command: list[str], execute: bool) -> dict[str, Any]:
    process = subprocess.run(
        command,
        cwd=ROOT,
        text=True,
        encoding="utf-8",
        errors="replace",
        capture_output=True,
        env=os.environ.copy(),
    )
    try:
        result = json.loads(process.stdout)
    except json.JSONDecodeError:
        result = {}
    return {
        "command": command,
        "returnCode": process.returncode,
        "result": result,
        "stderr": process.stderr[-2000:],
        "executed": execute,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--matrix", type=Path, default=DEFAULT_MATRIX)
    parser.add_argument("--provider", default="openai")
    parser.add_argument("--model", default="gpt-4o-mini")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--execute-controls", action="store_true")
    args = parser.parse_args()

    matrix = json.loads(args.matrix.read_text(encoding="utf-8"))
    campaign = args.matrix.parent / matrix["campaign"]
    if args.output is None:
        model_directory = "".join(
            character if character.isascii() and (character.isalnum() or character in "-_") else "_"
            for character in args.model
        )
        run_id = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
        args.output = Path("reports/agentdojo-matrices") / f"{matrix['name']}-{model_directory}-{run_id}"
    args.output.mkdir(parents=True, exist_ok=True)

    if args.execute_controls and not os.environ.get("OPENAI_API_KEY"):
        raise RuntimeError("OPENAI_API_KEY is not set in this terminal session.")

    commands = []
    for attack in matrix["attacks"]:
        output = args.output / attack
        command = [
            sys.executable,
            "benchmarks/agentdojo/campaign.py",
            "--campaign",
            str(campaign),
            "--attack",
            attack,
            "--provider",
            args.provider,
            "--model",
            args.model,
            "--trials",
            str(matrix["trialsPerScenario"]),
            "--max-llm-calls",
            str(matrix["maxLlmCallsPerTrial"]),
            "--output",
            str(output),
        ]
        if args.execute_controls:
            command.extend(["--execute-controls", "--limit", "4"])
        commands.append((attack, command))

    controls_planned = len(matrix["attacks"]) * 4 * matrix["trialsPerScenario"]
    results = []
    for attack, command in commands:
        item = run_command(command, args.execute_controls)
        item["attack"] = attack
        results.append(item)

    eligible = []
    for item in results:
        result_path = item["result"].get("results")
        if not result_path or not args.execute_controls:
            continue
        try:
            control_report = json.loads(Path(result_path).read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            continue
        for trial in control_report.get("eligibleForGoverned", []):
            eligible.append({"attack": item["attack"], **trial})

    report = {
        "matrix": matrix["name"],
        "model": args.model,
        "provider": args.provider,
        "controlsPlanned": controls_planned,
        "maximumLlmCalls": controls_planned * matrix["maxLlmCallsPerTrial"],
        "executedControls": controls_planned if args.execute_controls else 0,
        "eligibleForGoverned": eligible,
        "publicationReady": False,
        "publicationBlocker": "Matched governed trials have not been run and independently reviewed." if eligible else "No native control satisfied the injected task; this matrix has no safety-comparison denominator.",
        "runs": results,
    }
    report_path = args.output / f"{matrix['name']}-report.json"
    report_path.write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(json.dumps({"completed": args.execute_controls, "controlsPlanned": controls_planned, "executedControls": report["executedControls"], "eligibleForGoverned": eligible, "report": str(report_path.resolve())}, indent=2))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except RuntimeError as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(2)
