#!/usr/bin/env python3
"""Pre-registered, two-phase Tau3 native-versus-Kerna utility replication.

Phase one runs every native control. Phase two runs a Kerna gateway counterpart
only when its exact native control completed. This preserves the published
eligibility rule and prevents an unsuccessful control from inflating a Kerna
false-block or utility claim. The script writes only ignored raw reports; a
separate reviewed aggregate is required before public publication.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[2]
NATIVE_RUNNER = REPO_ROOT / "benchmarks" / "tau3" / "run_native.py"
GATEWAY_RUNNER = REPO_ROOT / "benchmarks" / "tau3" / "run_gateway.py"
TASK_ID = "0"


def read_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def simulation_summary(raw_path: Path) -> dict[str, Any]:
    raw = read_json(raw_path)
    simulation = raw["simulations"][0] if "simulations" in raw else raw
    reward_info = simulation.get("reward_info") or {}
    return {
        "reward": reward_info.get("reward"),
        "agentCostUsd": simulation.get("agent_cost"),
        "userCostUsd": simulation.get("user_cost"),
        "durationSeconds": simulation.get("duration"),
        "termination": simulation.get("termination_reason"),
    }


def execute(command: list[str]) -> tuple[int, str]:
    environment = os.environ.copy()
    # tau3 prints Unicode status markers. Explicit UTF-8 prevents a Windows
    # CP-1252 pipe from failing after the provider call has already completed.
    environment["PYTHONUTF8"] = "1"
    environment["PYTHONIOENCODING"] = "utf-8"
    completed = subprocess.run(
        command,
        cwd=REPO_ROOT,
        text=True,
        encoding="utf-8",
        errors="replace",
        env=environment,
        capture_output=True,
        check=False,
    )
    detail = ((completed.stdout or "") + (completed.stderr or "")).strip()
    return completed.returncode, detail[-2000:]


def base_report(args: argparse.Namespace, root: Path) -> dict[str, Any]:
    return {
        "benchmark": "tau3 retail task-0 native-versus-Kerna replication campaign",
        "version": 1,
        "executedAt": datetime.now(timezone.utc).isoformat(),
        "classification": "Replication campaign only. It is unpublished until reviewed; results must not be described as a universal Kerna utility or safety score.",
        "configuration": {
            "campaignId": args.campaign_id,
            "domain": "retail",
            "taskId": TASK_ID,
            "trials": args.trials,
            "seedStart": args.seed_start,
            "model": args.model,
            "maxSteps": args.max_steps,
            "nativeEligibility": "reward equals 1.0 for the exact native task and seed",
            "governedPolicy": {"default": "deny", "autoApprove": ["find_user_id_by_name_zip", "get_order_details", "get_product_details", "exchange_delivered_order_items"]},
            "campaignRoot": str(root.relative_to(REPO_ROOT)),
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--campaign-id", default="retail-task-0-replication-v1")
    parser.add_argument("--trials", type=int, default=20, help="pre-registered native controls; 20 is the minimum headline threshold")
    parser.add_argument("--seed-start", type=int, default=1000)
    parser.add_argument("--model", default="gpt-4o-mini")
    parser.add_argument("--max-steps", type=int, default=60)
    parser.add_argument("--execute-controls", action="store_true", help="run the bounded native-control phase")
    parser.add_argument("--execute-governed", action="store_true", help="run counterparts for completed native controls only")
    parser.add_argument("--force", action="store_true", help="replace an existing phase report; never use to silently pool runs")
    args = parser.parse_args()
    if args.trials < 1 or args.trials > 50:
        raise SystemExit("--trials must be 1..50")
    if args.execute_controls and args.execute_governed:
        raise SystemExit("Run phases separately: first --execute-controls, review, then --execute-governed.")

    root = REPO_ROOT / "reports" / "tau3" / "campaigns" / args.campaign_id
    controls_path = root / "native-controls.json"
    governed_path = root / "kerna-governed.json"
    plan = base_report(args, root)
    plan["controlsCommand"] = "python benchmarks\\tau3\\campaign.py --execute-controls"
    plan["governedCommand"] = "python benchmarks\\tau3\\campaign.py --execute-governed"
    if not args.execute_controls and not args.execute_governed:
        plan["status"] = "planned"
        print(json.dumps(plan, indent=2))
        return 0

    root.mkdir(parents=True, exist_ok=True)
    if args.execute_controls:
        if controls_path.exists() and not args.force:
            raise SystemExit(f"{controls_path} already exists. Review it or use a new --campaign-id; do not silently pool reruns.")
        records: list[dict[str, Any]] = []
        for index in range(args.trials):
            seed = args.seed_start + index
            wrapper = root / "native" / f"trial-{index:02d}.json"
            reused = wrapper.exists() and read_json(wrapper).get("status") == "completed"
            if reused:
                code, detail = 0, ""
            else:
                code, detail = execute(
                    [
                        sys.executable, str(NATIVE_RUNNER), "--execute", "--task-ids", TASK_ID,
                        "--model", args.model, "--max-steps", str(args.max_steps), "--seed", str(seed), "--out", str(wrapper),
                    ]
                )
            record: dict[str, Any] = {"trial": index, "seed": seed, "returnCode": code, "wrapper": str(wrapper.relative_to(REPO_ROOT)), "reusedCompletedRun": reused}
            if code == 0 and wrapper.exists():
                wrapper_data = read_json(wrapper)
                raw_path = REPO_ROOT / wrapper_data["resultsPath"]
                record.update(simulation_summary(raw_path))
                record["rawResult"] = str(raw_path.relative_to(REPO_ROOT))
                record["eligibleForGoverned"] = record["reward"] == 1.0
            else:
                record.update({"eligibleForGoverned": False, "errorTail": detail})
            records.append(record)
            print(json.dumps({"phase": "native", "trial": index, "seed": seed, "eligibleForGoverned": record["eligibleForGoverned"]}))
        report = base_report(args, root)
        eligible = sum(1 for record in records if record["eligibleForGoverned"])
        report.update({"status": "completed", "phase": "native-controls", "records": records, "completed": sum(1 for record in records if record["returnCode"] == 0), "eligibleForGoverned": eligible, "publicationReady": False})
        controls_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        print(json.dumps({"completed": True, "controlsRun": len(records), "eligibleForGoverned": eligible, "results": str(controls_path)}, indent=2))
        return 0

    if not controls_path.exists():
        raise SystemExit(f"Native controls are missing: {controls_path}. Run --execute-controls first.")
    controls = read_json(controls_path)
    previous_records: dict[int, dict[str, Any]] = {}
    if governed_path.exists() and not args.force:
        previous_records = {record["trial"]: record for record in read_json(governed_path).get("records", [])}
    records: list[dict[str, Any]] = []
    for control in controls["records"]:
        if not control.get("eligibleForGoverned"):
            continue
        index, seed = control["trial"], control["seed"]
        wrapper = root / "governed" / f"trial-{index:02d}.json"
        existing = previous_records.get(index)
        reused = bool(existing and existing.get("returnCode") == 0 and wrapper.exists() and read_json(wrapper).get("status") == "completed")
        if reused:
            records.append({**existing, "reusedCompletedRun": True})
            print(json.dumps({"phase": "governed", "trial": index, "seed": seed, "reward": existing.get("reward"), "reusedCompletedRun": True}))
            continue
        code, detail = execute(
            [
                sys.executable, str(GATEWAY_RUNNER), "--execute", "--model", args.model,
                "--max-steps", str(args.max_steps), "--seed", str(seed), "--out", str(wrapper),
            ]
        )
        record: dict[str, Any] = {"trial": index, "seed": seed, "returnCode": code, "wrapper": str(wrapper.relative_to(REPO_ROOT)), "nativeReward": control.get("reward")}
        if code == 0 and wrapper.exists():
            wrapper_data = read_json(wrapper)
            raw_path = REPO_ROOT / wrapper_data["rawResultPath"]
            governed_summary = simulation_summary(raw_path)
            record.update({
                **governed_summary,
                "receiptComplete": wrapper_data.get("receiptComplete"),
                "policyBlocks": (wrapper_data.get("toolCalls") or {}).get("blocked"),
                "rawResult": wrapper_data.get("rawResultPath"),
            })
        else:
            record["errorTail"] = detail
        records.append(record)
        print(json.dumps({"phase": "governed", "trial": index, "seed": seed, "reward": record.get("reward")}))
    report = base_report(args, root)
    completed = [record for record in records if record["returnCode"] == 0]
    successful = [record for record in completed if record.get("reward") == 1.0]
    report.update(
        {
            "status": "completed",
            "phase": "kerna-governed",
            "nativeEligible": sum(1 for record in controls["records"] if record.get("eligibleForGoverned")),
            "records": records,
            "completed": len(completed),
            "utilityRetained": len(successful),
            "utilityRetentionRate": len(successful) / len(records) if records else None,
            "receiptComplete": sum(1 for record in completed if record.get("receiptComplete")),
            "policyBlocks": sum((record.get("policyBlocks") or 0) for record in completed),
            "publicationReady": len(records) >= 20 and len(completed) == len(records),
        }
    )
    governed_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"completed": True, "governedRun": len(records), "utilityRetained": len(successful), "publicationReady": report["publicationReady"], "results": str(governed_path)}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
