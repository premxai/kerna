#!/usr/bin/env python3
"""Run the eligible tau3 retail task through Kerna's MCP policy gateway.

This is a matched utility *trial*, not a publishable aggregate.  The native
calibration pre-registered tasks 0, 1, and 2.  Only task 0 completed natively,
so it is the sole eligible Kerna counterpart under the published comparison
rule.  The adapter preserves tau3's one in-memory environment: Kerna's
downstream MCP bridge returns each tool call to that exact environment for
execution and evaluation.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import sqlite3
import sys
import uuid
from datetime import datetime, timezone
from pathlib import Path
from types import MethodType
from typing import Any

from gateway_adapter import AdapterError, GatewayClient, LoopbackBridgeServer, REPO_ROOT

TAU_ROOT = REPO_ROOT / "reports" / "tau3-source"
TASK_ID = "0"
TASK_TOOLS = [
    "find_user_id_by_name_zip",
    "get_order_details",
    "get_product_details",
    "exchange_delivered_order_items",
]


def mcp_schemas(orchestrator: Any) -> list[dict[str, Any]]:
    """Translate the exact tau3 tool definitions without changing the agent schema."""
    schemas: list[dict[str, Any]] = []
    for tool in orchestrator.environment.get_tools():
        function = tool.openai_schema["function"]
        schemas.append(
            {
                "name": function["name"],
                "description": function.get("description", ""),
                "inputSchema": function["parameters"],
            }
        )
    return schemas


def compact_schemas(schemas: list[dict[str, Any]]) -> list[dict[str, Any]]:
    return [{"name": item["name"], "inputSchema": item["inputSchema"]} for item in schemas]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--execute", action="store_true", help="make the one bounded provider-backed matched trial")
    parser.add_argument("--model", default="gpt-4o-mini", help="same model for tau3 agent and user simulator")
    parser.add_argument("--max-steps", type=int, default=60, help="must match the corrected native control")
    parser.add_argument("--kerna", default="target/debug/kerna.exe", help="Kerna executable relative to repository root")
    parser.add_argument("--out", default="reports/tau3/gateway-task-0.json", help="redacted wrapper report path")
    args = parser.parse_args()
    if not 20 <= args.max_steps <= 200:
        raise SystemExit("--max-steps must be 20..200")
    if not TAU_ROOT.is_dir():
        raise SystemExit("Pinned tau3 checkout is missing. Run benchmarks/tau3/preflight.py first.")
    kerna = (REPO_ROOT / args.kerna).resolve()
    if not kerna.is_file():
        raise SystemExit(f"Kerna executable not found: {kerna}. Run `cargo build` from the repository root first.")
    if shutil.which("uv") is None:
        raise SystemExit("uv is required for the pinned tau3 checkout")

    run_id = "kerna-gateway-retail-task-0-" + datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    run_dir = REPO_ROOT / "reports" / "tau3" / "gateway-runs" / run_id
    report: dict[str, Any] = {
        "benchmark": "tau3 Kerna gateway matched utility trial",
        "version": 1,
        "executedAt": datetime.now(timezone.utc).isoformat(),
        "classification": "One eligible matched utility trial only. It is not a public aggregate, safety score, or prevention claim.",
        "configuration": {
            "tau3Domain": "retail",
            "taskId": TASK_ID,
            "selectionRule": "Exact counterpart only because this task completed in the pre-registered native calibration; tasks 1 and 2 did not.",
            "agentModel": args.model,
            "userModel": args.model,
            "maxSteps": args.max_steps,
            "maxErrors": 5,
            "timeoutSeconds": 300,
            "seed": 300,
            "policy": {"default": "deny", "autoApprove": TASK_TOOLS},
        },
    }
    output = (REPO_ROOT / args.out).resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    if not args.execute:
        report["status"] = "planned"
        report["nextCommand"] = "Re-run with --execute from a terminal that already has OPENAI_API_KEY set. Run gateway_contract_test.py first."
        output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        print(json.dumps(report, indent=2))
        return 0
    if not os.environ.get("OPENAI_API_KEY"):
        raise SystemExit("OPENAI_API_KEY is required in the executing terminal. Do not place it in a committed file.")

    # Re-exec under tau3's pinned environment so the public command remains the
    # same as the native runner: `python benchmarks/tau3/run_gateway.py
    # --execute`.  Avoid silently falling back to a globally installed tau2.
    try:
        from tau2.data_model.message import ToolCall, ToolMessage
        from tau2.data_model.simulation import TextRunConfig
        from tau2.runner.build import build_text_orchestrator
        from tau2.runner.helpers import get_tasks
        from tau2.runner.simulation import run_simulation
    except ImportError as error:
        environment = os.environ.copy()
        environment["PYTHONUTF8"] = "1"
        environment["PYTHONIOENCODING"] = "utf-8"
        command = [shutil.which("uv") or "uv", "run", "python", str(Path(__file__).resolve()), *sys.argv[1:]]
        return __import__("subprocess").run(command, cwd=TAU_ROOT, env=environment, check=False).returncode

    config = TextRunConfig(
        domain="retail",
        task_ids=[TASK_ID],
        agent="llm_agent",
        llm_agent=args.model,
        user="user_simulator",
        llm_user=args.model,
        num_trials=1,
        max_steps=args.max_steps,
        max_errors=5,
        timeout=300,
        max_concurrency=1,
        seed=300,
    )
    task = get_tasks("retail", task_ids=[TASK_ID])[0]
    orchestrator = build_text_orchestrator(config, task, seed=300, simulation_id=str(uuid.uuid4()))
    source_schemas = mcp_schemas(orchestrator)
    active_call: ToolCall | None = None

    def invoke_same_environment(name: str, arguments: dict[str, Any]) -> dict[str, Any]:
        if active_call is None:
            raise AdapterError("bridge received a tool call outside tau3 orchestration")
        if name != active_call.name or arguments != active_call.arguments:
            raise AdapterError("bridge request does not match tau3's active tool call")
        return orchestrator.environment.get_response(active_call).model_dump(mode="json")

    run_dir.mkdir(parents=True, exist_ok=True)
    with LoopbackBridgeServer(lambda: source_schemas, invoke_same_environment) as bridge:
        gateway = GatewayClient.start(
            kerna=kerna,
            run_dir=run_dir,
            endpoint=bridge.endpoint,
            token=bridge.token,
            allowed_tools=TASK_TOOLS,
        )
        try:
            exposed_schemas = gateway.list_tools()
            if compact_schemas(exposed_schemas) != compact_schemas(source_schemas):
                raise AdapterError("gateway tool schemas differ from the native tau3 environment")

            def execute_via_gateway(self: Any, tool_calls: list[ToolCall]) -> list[ToolMessage]:
                nonlocal active_call
                results: list[ToolMessage] = []
                for tool_call in tool_calls:
                    active_call = tool_call
                    try:
                        response = gateway.call_tool(tool_call.name, tool_call.arguments)
                    finally:
                        active_call = None
                    blocks = response.get("content") or []
                    text = blocks[0].get("text", "") if blocks else ""
                    if response.get("isError"):
                        tool_result = ToolMessage(
                            id=tool_call.id,
                            role="tool",
                            content=text,
                            requestor=tool_call.requestor,
                            error=True,
                        )
                    else:
                        tool_result = ToolMessage.model_validate(json.loads(text))
                        if tool_result.id != tool_call.id or tool_result.requestor != tool_call.requestor:
                            raise AdapterError("bridge changed tau3 tool-call identity")
                    if tool_result.error:
                        self.num_errors += 1
                    results.append(tool_result)
                return results

            # Keep tau3's agent, user, task, environment, and evaluator intact;
            # replace only the direct environment execution method.
            orchestrator._execute_tool_calls = MethodType(execute_via_gateway, orchestrator)
            simulation = run_simulation(orchestrator)
            task_id = gateway.task_id
        finally:
            gateway.close()

    events: list[str] = []
    if task_id:
        database = run_dir / "kerna-gateway.db"
        with sqlite3.connect(database) as connection:
            events = [row[0] for row in connection.execute("select event_type from events where task_id = ? order by sequence", (task_id,))]
    raw_result = run_dir / "tau3-result.json"
    raw_result.write_text(json.dumps(simulation.model_dump(mode="json"), indent=2) + "\n", encoding="utf-8")
    reward = simulation.reward_info.reward if simulation.reward_info else None
    requested = events.count("tool.call.requested")
    completed = events.count("tool.call.completed")
    policy_checked = events.count("tool.policy.checked")
    report.update(
        {
            "status": "completed",
            "kernaGatewayTaskId": task_id,
            "reward": reward,
            "agentCostUsd": simulation.agent_cost,
            "userCostUsd": simulation.user_cost,
            "toolCalls": {"requested": requested, "policyChecked": policy_checked, "completed": completed, "blocked": events.count("tool.call.blocked")},
            "receiptComplete": requested > 0 and requested == completed == policy_checked,
            "rawResultPath": str(raw_result.relative_to(REPO_ROOT)),
            "gatewayRunPath": str(run_dir.relative_to(REPO_ROOT)),
        }
    )
    output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
