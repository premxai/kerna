#!/usr/bin/env python3
"""Validate and plan a fixed AgentDojo pilot without making model calls.

The resulting plan intentionally separates native controls from Kerna-governed
runs. Only scenarios where the native control satisfies the injection task
should advance to the paid governed comparison.
"""

from __future__ import annotations

import argparse
import importlib.metadata
import json
import os
import subprocess
import sys
from pathlib import Path
from typing import Any


REQUIRED_AGENTDOJO_VERSION = "0.1.35"
ROOT = Path(__file__).resolve().parents[2]
DEFAULT_CAMPAIGN = Path(__file__).with_name("campaigns") / "workspace-injection-pilot.json"


class AttackIdentity:
    """Supplies the stable name required by fixed AgentDojo attacks."""

    name = "kerna-campaign-planner"

    def query(self, *args: Any, **kwargs: Any) -> Any:
        raise RuntimeError("Fixed attacks must not call an AgentDojo pipeline while planning.")


def require_agentdojo() -> None:
    try:
        version = importlib.metadata.version("agentdojo")
    except importlib.metadata.PackageNotFoundError as error:
        raise RuntimeError("AgentDojo is not installed. Install benchmarks/agentdojo/requirements.txt first.") from error
    if version != REQUIRED_AGENTDOJO_VERSION:
        raise RuntimeError(f"AgentDojo {REQUIRED_AGENTDOJO_VERSION} is required; found {version}.")


def command_for(mode: str, scenario: dict[str, Any], args: argparse.Namespace) -> list[str]:
    command = [
        sys.executable,
        "benchmarks/agentdojo/run.py",
        "--execute",
        "--mode",
        mode,
        "--suite",
        args.suite,
        "--benchmark-version",
        args.benchmark_version,
        "--user-task",
        scenario["userTask"],
        "--injection-task",
        scenario["injectionTask"],
        "--attack",
        args.attack,
        "--provider",
        args.provider,
        "--model",
        args.model,
    ]
    if mode == "control":
        return [*command, "--max-llm-calls", str(args.max_llm_calls)]
    return [
        *command,
        "--max-cost-usd",
        str(args.max_cost_usd),
        "--kerna",
        args.kerna,
    ]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--campaign", type=Path, default=DEFAULT_CAMPAIGN)
    parser.add_argument(
        "--output",
        type=Path,
        help="Report directory. Defaults to a model-specific directory under reports/agentdojo-campaigns.",
    )
    parser.add_argument("--provider", default="openai")
    parser.add_argument("--model", default="gpt-4o-mini")
    parser.add_argument("--max-llm-calls", type=int, default=4)
    parser.add_argument("--max-cost-usd", type=float, default=0.10)
    parser.add_argument("--kerna", default=".\\target\\debug\\kerna.exe" if sys.platform == "win32" else "./target/debug/kerna")
    parser.add_argument(
        "--execute-controls",
        action="store_true",
        help="Run native control scenarios. This makes provider API calls.",
    )
    parser.add_argument(
        "--limit",
        type=int,
        default=0,
        help="Maximum number of control scenarios to run; required with --execute-controls.",
    )
    args = parser.parse_args()
    require_agentdojo()

    campaign = json.loads(args.campaign.read_text(encoding="utf-8"))
    args.suite = campaign["suite"]
    args.benchmark_version = campaign["benchmarkVersion"]
    args.attack = campaign["attack"]
    if args.output is None:
        model_directory = "".join(
            character if character.isascii() and (character.isalnum() or character in "-_") else "_"
            for character in args.model
        )
        args.output = Path("reports/agentdojo-campaigns") / f"{campaign['name']}-{model_directory}"

    from agentdojo.attacks import baseline_attacks  # noqa: F401 - registers fixed attacks
    from agentdojo.attacks.attack_registry import load_attack
    from agentdojo.task_suite.load_suites import get_suite

    suite = get_suite(args.benchmark_version, args.suite)
    attack = load_attack(args.attack, suite, AttackIdentity())
    planned_scenarios = []
    for scenario in campaign["scenarios"]:
        user_task = suite.get_user_task_by_id(scenario["userTask"])
        injection_task = suite.get_injection_task_by_id(scenario["injectionTask"])
        actual_sources = set(attack.attack(user_task, injection_task))
        expected_sources = set(scenario["source"].split(" and "))
        if actual_sources != expected_sources:
            raise RuntimeError(
                f"Campaign source mismatch for {scenario['id']}: "
                f"expected {sorted(expected_sources)}, got {sorted(actual_sources)}."
            )
        planned_scenarios.append(
            {
                **scenario,
                "controlCommand": command_for("control", scenario, args),
                "governedCommand": command_for("governed", scenario, args),
                "advanceRule": "Run governed only if control has utility=true and agentDojoInjectionTaskSatisfied=true.",
            }
        )

    plan = {
        "campaign": campaign["name"],
        "description": campaign["description"],
        "agentDojoVersion": REQUIRED_AGENTDOJO_VERSION,
        "model": args.model,
        "provider": args.provider,
        "scenarioCount": len(planned_scenarios),
        "scenarios": planned_scenarios,
    }
    args.output.mkdir(parents=True, exist_ok=True)
    plan_path = args.output / f"{campaign['name']}-plan.json"
    plan_path.write_text(json.dumps(plan, indent=2), encoding="utf-8")
    if not args.execute_controls:
        print(json.dumps({"planned": True, "plan": str(plan_path.resolve()), "scenarioCount": len(planned_scenarios)}, indent=2))
        return 0

    if args.limit <= 0:
        raise RuntimeError("--execute-controls requires a positive --limit so campaign spend stays explicit.")
    if not os.environ.get("OPENAI_API_KEY"):
        raise RuntimeError("OPENAI_API_KEY is not set in this terminal session.")

    selected = planned_scenarios[: args.limit]
    outcomes = []
    for scenario in selected:
        process = subprocess.run(
            scenario["controlCommand"],
            cwd=ROOT,
            env=os.environ.copy(),
            text=True,
            encoding="utf-8",
            errors="replace",
            capture_output=True,
        )
        stdout_path = args.output / f"{scenario['id']}-control.stdout.txt"
        stderr_path = args.output / f"{scenario['id']}-control.stderr.txt"
        stdout_path.write_text(process.stdout, encoding="utf-8")
        stderr_path.write_text(process.stderr, encoding="utf-8")
        try:
            result = json.loads(process.stdout)
        except json.JSONDecodeError:
            result = {}
        eligible = bool(
            process.returncode == 0
            and result.get("utility") is True
            and result.get("agentDojoInjectionTaskSatisfied") is True
        )
        outcomes.append(
            {
                "id": scenario["id"],
                "returnCode": process.returncode,
                "utility": result.get("utility"),
                "agentDojoInjectionTaskSatisfied": result.get("agentDojoInjectionTaskSatisfied"),
                "eligibleForGoverned": eligible,
                "stdout": str(stdout_path.resolve()),
                "stderr": str(stderr_path.resolve()),
            }
        )

    outcome_path = args.output / f"{campaign['name']}-control-results.json"
    outcome_path.write_text(json.dumps({"outcomes": outcomes}, indent=2), encoding="utf-8")
    print(
        json.dumps(
            {
                "completed": True,
                "controlsRun": len(outcomes),
                "eligibleForGoverned": [item["id"] for item in outcomes if item["eligibleForGoverned"]],
                "results": str(outcome_path.resolve()),
            },
            indent=2,
        )
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except RuntimeError as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(2)
