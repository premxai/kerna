#!/usr/bin/env python3
"""Run one bounded ToolEmu case through Kerna and its upstream simulator.

Default mode prints a plan only. ``--execute`` makes provider calls from both
Kerna (the agent) and ToolEmu (the virtual-tool simulator), so it is intended
for an explicitly reviewed, small permissive/governed pilot only.
"""

from __future__ import annotations

import argparse
import json
import os
import sqlite3
import subprocess
import sys
import tempfile
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

from gateway_adapter import BRIDGE_PATH, REPO_ROOT, LoopbackBridgeServer, ToolEmuCaseEnvironment, mcp_tool_name
from upstream_simulator import UpstreamToolEmuSimulator


def write_config(run_dir: Path, args: argparse.Namespace, allowed_tools: list[str]) -> Path:
    bridge_args = [str(BRIDGE_PATH), "--endpoint", args.endpoint, "--token", args.token]
    permissions = "\n\n".join(f'[[permissions]]\ntool = {json.dumps(tool)}\naction = "auto_approve"' for tool in allowed_tools)
    config = run_dir / "kerna.toml"
    config.write_text(
        "\n".join([
            'llm_provider = "openai"', f"llm_model = {json.dumps(args.agent_model)}",
            'db_path = "kerna.db"', 'sandbox_dir = "sandbox"', 'memory_backend = "sqlite"',
            f"max_runtime_seconds = {args.max_runtime_seconds}", f"max_tool_calls = {args.max_tool_calls}",
            f"max_llm_calls = {args.max_llm_calls}", f"max_cost_usd = {args.max_cost_usd}", "max_memory_writes = 0", "",
            "[[mcp_servers]]", 'name = "toolemu-loopback"', f"command = {json.dumps(sys.executable)}",
            "args = [" + ", ".join(json.dumps(value) for value in bridge_args) + "]", "",
            permissions, "",
        ]),
        encoding="utf-8",
    )
    return config


def case_by_name(assets: Path, name: str) -> dict[str, Any]:
    cases = json.loads((assets / "all_cases.json").read_text(encoding="utf-8"))
    case = next((item for item in cases if item.get("name") == name), None)
    if case is None:
        raise RuntimeError(f"ToolEmu case not found: {name}")
    return case


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--case", default="official_0")
    parser.add_argument("--arm", choices=["permissive", "governed"], default="permissive")
    parser.add_argument("--agent-model", default="gpt-4o-mini")
    parser.add_argument("--simulator-model", default="gpt-4o-mini")
    parser.add_argument("--allow-tool", action="append", default=[])
    parser.add_argument("--max-runtime-seconds", type=int, default=120)
    parser.add_argument("--max-tool-calls", type=int, default=4)
    parser.add_argument("--max-llm-calls", type=int, default=4)
    parser.add_argument("--max-simulator-calls", type=int, default=4)
    parser.add_argument("--max-cost-usd", type=float, default=0.10, help="Kerna agent cost guard only; excludes ToolEmu simulator calls")
    parser.add_argument("--kerna", default=str(REPO_ROOT / "target" / "debug" / ("kerna.exe" if sys.platform == "win32" else "kerna")))
    parser.add_argument("--assets", type=Path, default=REPO_ROOT / "reports" / "toolemu-source" / "assets")
    parser.add_argument("--output", type=Path, default=REPO_ROOT / "reports" / "toolemu" / "runs")
    parser.add_argument("--execute", action="store_true")
    args = parser.parse_args()
    assets = args.assets.resolve()
    case = case_by_name(assets, args.case)
    schemas = ToolEmuCaseEnvironment.from_assets(assets, args.case, lambda _request: None).tools()
    all_tools = [schema["name"] for schema in schemas]
    search = mcp_tool_name("Todoist", "SearchTasks")
    allowed_tools = all_tools if args.arm == "permissive" else list(dict.fromkeys(args.allow_tool or [search]))
    invalid = sorted(set(allowed_tools) - set(all_tools))
    if invalid:
        raise RuntimeError(f"policy names are not exposed by this case: {', '.join(invalid)}")
    plan = {
        "benchmark": "ToolEmu Kerna gateway pilot", "version": 1, "executedAt": datetime.now(UTC).isoformat(),
        "classification": "Matched Kerna permissive/governed pilot only; not an upstream ToolEmu leaderboard result or a native-agent comparison.",
        "case": args.case, "userInstruction": case["User Instruction"], "toolkits": case["Toolkits"], "arm": args.arm,
        "agentModel": args.agent_model, "simulatorModel": args.simulator_model, "toolsExposed": all_tools, "allowedTools": allowed_tools,
        "budgets": {"maxRuntimeSeconds": args.max_runtime_seconds, "maxToolCalls": args.max_tool_calls, "maxLlmCalls": args.max_llm_calls, "maxSimulatorCalls": args.max_simulator_calls, "maxAgentCostUsd": args.max_cost_usd},
        "providerCallsAuthorized": args.execute,
    }
    if not args.execute:
        print(json.dumps({"dryRun": True, "plan": plan}, indent=2))
        return 0
    if not Path(args.kerna).is_file():
        raise RuntimeError(f"Kerna executable not found: {args.kerna}")
    if not os.environ.get("OPENAI_API_KEY"):
        raise RuntimeError("OPENAI_API_KEY must be set in this terminal before --execute.")

    from toolemu.utils.llm import ChatOpenAI

    simulator = UpstreamToolEmuSimulator(assets, args.case, ChatOpenAI(model_name=args.simulator_model, temperature=0), max_calls=args.max_simulator_calls)
    environment = ToolEmuCaseEnvironment.from_assets(assets, args.case, simulator.observe)
    with tempfile.TemporaryDirectory(prefix="kerna-toolemu-run-") as temporary:
        run_dir = Path(temporary)
        with LoopbackBridgeServer(environment) as bridge:
            args.endpoint, args.token = bridge.endpoint, bridge.token
            config = write_config(run_dir, args, allowed_tools)
            # Kerna gets its key through the normal environment; the bridge
            # scrubs it immediately on startup and never needs a credential.
            process_env = os.environ.copy()
            process_env["KERNA_LLM_API_KEY"] = process_env["OPENAI_API_KEY"]
            execution = subprocess.run([args.kerna, "run", case["User Instruction"]], cwd=run_dir, env=process_env, text=True, encoding="utf-8", errors="replace", capture_output=True)
        # sqlite3.Connection's context manager commits/rolls back but does not
        # close the handle. Explicitly close it before TemporaryDirectory
        # cleanup, otherwise Windows retains a lock on kerna.db.
        connection = sqlite3.connect(run_dir / "kerna.db")
        try:
            events = [
                {"eventType": row[0], "tool": row[1], "policyDecision": row[2]}
                for row in connection.execute("select event_type, tool, policy_decision from events order by sequence")
            ]
            task = connection.execute("select id, status, result_text from tasks order by created_at desc limit 1").fetchone()
        finally:
            connection.close()
        result = {
            **plan, "status": "completed" if execution.returncode == 0 else "failed", "returnCode": execution.returncode,
            "taskId": task[0] if task else None, "taskStatus": task[1] if task else None, "modelOutput": task[2] if task else None,
            "toolEmuTrace": environment.trace, "toolEmuSimulatorCalls": simulator.calls, "receiptEvents": events,
            "kernaStdout": execution.stdout[-4000:], "kernaStderr": execution.stderr[-4000:],
            "limitations": "ToolEmu simulator cost is capped by call count but is not included in Kerna's maxAgentCostUsd receipt.",
        }
    output = args.output.resolve() / f"{args.case}-{args.arm}-{datetime.now(UTC).strftime('%Y%m%dT%H%M%SZ')}.json"
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result, indent=2))
    return 0 if execution.returncode == 0 else execution.returncode


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except RuntimeError as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(2)
