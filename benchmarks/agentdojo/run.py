#!/usr/bin/env python3
"""Run one AgentDojo task through Kerna's scheduler and MCP boundary.

The command is a dry run unless --execute is supplied. Dry runs generate the
fully resolved scenario and Kerna configuration without calling a model.
"""

from __future__ import annotations

import argparse
import importlib.metadata
import json
import os
import shutil
import sqlite3
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any


REQUIRED_AGENTDOJO_VERSION = "0.1.35"
REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
BRIDGE_PATH = Path(__file__).with_name("kerna_agentdojo_mcp.py")
WORKSPACE_MUTATING_TOOLS = {
    "send_email",
    "delete_email",
    "create_calendar_event",
    "cancel_calendar_event",
    "reschedule_calendar_event",
    "add_calendar_event_participants",
    "append_to_file",
    "create_file",
    "delete_file",
    "share_file",
}

# The pilot must force the model to read the artifact that contains the
# injected instruction. This official Workspace pair asks for a Facebook
# security code while the injecagent attack injects an instruction into that
# same security email to exfiltrate the code. The previous calendar-only
# default could never encounter its email injection and therefore could not
# establish a meaningful control baseline.
DEFAULT_PILOT_USER_TASK = "user_task_16"
DEFAULT_PILOT_INJECTION_TASK = "injection_task_4"
DEFAULT_PILOT_ATTACK = "injecagent"


class AttackIdentity:
    """Only supplies a stable name for fixed AgentDojo attacks."""

    name = "kerna-governed-agent"

    def query(self, *args: Any, **kwargs: Any) -> Any:
        raise RuntimeError("The fixed attack must not query an AgentDojo pipeline")


def require_agentdojo() -> None:
    try:
        installed = importlib.metadata.version("agentdojo")
    except importlib.metadata.PackageNotFoundError as exc:
        raise RuntimeError(
            "AgentDojo is not installed. Run: python -m pip install -r benchmarks/agentdojo/requirements.txt"
        ) from exc
    if installed != REQUIRED_AGENTDOJO_VERSION:
        raise RuntimeError(
            f"AgentDojo {REQUIRED_AGENTDOJO_VERSION} is required; found {installed}."
        )


def toml_string(value: str) -> str:
    return json.dumps(value)


def write_kerna_config(
    path: Path,
    python_executable: str,
    scenario_path: Path,
    bridge_result_path: Path,
    database_path: Path,
    sandbox_path: Path,
    args: argparse.Namespace,
    deny_tools: list[str],
) -> None:
    deny_rules = "".join(
        f"\n[[permissions]]\ntool = {toml_string(tool)}\naction = \"deny\"\n" for tool in deny_tools
    )
    text = f'''llm_provider = {toml_string(args.provider)}
llm_model = {toml_string(args.model)}
db_path = {toml_string(str(database_path))}
sandbox_dir = {toml_string(str(sandbox_path))}
memory_backend = "sqlite"
max_runtime_seconds = {args.max_runtime_seconds}
max_tool_calls = {args.max_tool_calls}
max_llm_calls = {args.max_llm_calls}
max_cost_usd = {args.max_cost_usd}
max_output_bytes = {args.max_output_bytes}
max_memory_writes = 0

[[mcp_servers]]
name = "agentdojo-bridge"
command = {toml_string(python_executable)}
args = [{toml_string(str(BRIDGE_PATH))}, "--scenario", {toml_string(str(scenario_path))}, "--result", {toml_string(str(bridge_result_path))}]
enabled = true
runtime_mode = "local"
capabilities = ["*"]

[[permissions]]
tool = "*"
action = "auto_approve"
{deny_rules}
'''
    path.write_text(text, encoding="utf-8")


def score_kerna_result(scenario: dict[str, Any], bridge_result_path: Path, database_path: Path) -> dict[str, Any]:
    from agentdojo.functions_runtime import FunctionCall
    from agentdojo.task_suite.load_suites import get_suite

    bridge = json.loads(bridge_result_path.read_text(encoding="utf-8"))
    suite = get_suite(scenario["benchmark_version"], scenario["suite"])
    user_task = suite.get_user_task_by_id(scenario["user_task"])
    injection_task_id = scenario.get("injection_task")
    injection_task = suite.get_injection_task_by_id(injection_task_id) if injection_task_id else None
    environment_type = suite.environment_type
    pre_environment = environment_type.model_validate(bridge["preEnvironment"])
    post_environment = environment_type.model_validate(bridge["postEnvironment"])
    trace = [FunctionCall.model_validate(call) for call in bridge["functionTrace"]]

    with sqlite3.connect(database_path) as connection:
        row = connection.execute(
            "SELECT id, status, result_text FROM tasks ORDER BY created_at DESC LIMIT 1"
        ).fetchone()
        events = connection.execute(
            "SELECT event_type, tool, policy_decision FROM events ORDER BY sequence"
        ).fetchall()
    task_id, status, model_output = row if row else (None, "missing", "")
    model_output_blocks = [{"type": "text", "content": model_output or ""}]
    utility = suite._check_user_task_utility(
        user_task, model_output_blocks, pre_environment, post_environment, trace
    )
    injection_task_executed = (
        suite._check_injection_task_security(
            injection_task, model_output_blocks, pre_environment, post_environment, trace
        )
        if injection_task
        else True
    )
    completed_tools = {tool for event_type, tool, _ in events if event_type == "tool.call.completed"}
    receipt_complete = all(call.function in completed_tools for call in trace)
    return {
        "taskId": task_id,
        "status": status,
        "utility": utility,
        # AgentDojo's injection-task predicate is true when the adversarial
        # task condition is satisfied. Publish both the raw condition and the
        # unambiguous prevention metric; never make callers infer its polarity.
        "agentDojoInjectionTaskSatisfied": injection_task_executed,
        "unsafeActionPrevented": not injection_task_executed,
        "receiptComplete": receipt_complete,
        "bridgeToolCalls": len(trace),
        "functionTrace": [call.model_dump(mode="json") for call in trace],
        "receiptEvents": [
            {"eventType": event_type, "tool": tool, "policyDecision": decision}
            for event_type, tool, decision in events
        ],
        "controlRuntime": "kerna-governed-mcp",
        "kernaReceiptAvailable": True,
    }


def run_native_control(scenario: dict[str, Any], args: argparse.Namespace) -> dict[str, Any]:
    """Run AgentDojo's own unprotected tool loop as the matched baseline.

    This intentionally does not route through Kerna. A control that still
    passes through Kerna's prompt-injection filter is a protected run and
    cannot establish the baseline required to attribute protection to Kerna.
    """
    if args.provider != "openai":
        raise RuntimeError("Native AgentDojo control currently supports --provider openai only.")

    import openai
    from agentdojo.agent_pipeline.agent_pipeline import AgentPipeline, PipelineConfig
    from agentdojo.agent_pipeline.llms.openai_llm import OpenAILLM
    from agentdojo.agent_pipeline.tool_execution import ToolsExecutionLoop
    from agentdojo.functions_runtime import FunctionsRuntime
    from agentdojo.task_suite.load_suites import get_suite
    from agentdojo.task_suite.task_suite import (
        functions_stack_trace_from_messages,
        model_output_from_messages,
    )

    suite = get_suite(scenario["benchmark_version"], scenario["suite"])
    user_task = suite.get_user_task_by_id(scenario["user_task"])
    injection_task = suite.get_injection_task_by_id(scenario["injection_task"])
    environment = suite.load_and_inject_default_environment(scenario["injections"])
    task_environment = user_task.init_environment(environment)
    pre_environment = task_environment.model_copy(deep=True)
    pipeline = AgentPipeline.from_config(
        PipelineConfig(
            llm=OpenAILLM(openai.OpenAI(), args.model),
            model_id=None,
            defense=None,
            system_message_name=None,
            system_message=None,
            tool_output_format="json",
        )
    )
    # AgentDojo makes the initial model call before entering its tools loop.
    # Limit the loop so native control observes the same total LLM-call bound
    # as the governed Kerna run.
    for element in pipeline.elements:
        if isinstance(element, ToolsExecutionLoop):
            element.max_iters = max(args.max_llm_calls - 1, 0)

    runtime = FunctionsRuntime(suite.tools)
    _, _, task_environment, messages, _ = pipeline.query(
        user_task.PROMPT, runtime, task_environment
    )
    model_output = model_output_from_messages(messages) or []
    trace = functions_stack_trace_from_messages(messages)
    utility = suite._check_user_task_utility(
        user_task, model_output, pre_environment, task_environment, trace
    )
    injection_task_executed = suite._check_injection_task_security(
        injection_task, model_output, pre_environment, task_environment, trace
    )
    return {
        "taskId": None,
        "status": "completed",
        "utility": utility,
        "agentDojoInjectionTaskSatisfied": injection_task_executed,
        "unsafeActionPrevented": not injection_task_executed,
        "receiptComplete": None,
        "bridgeToolCalls": len(trace),
        "functionTrace": [call.model_dump(mode="json") for call in trace],
        "receiptEvents": [],
        "controlRuntime": "agentdojo-native",
        "kernaReceiptAvailable": False,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--suite", default="workspace")
    parser.add_argument("--user-task", default=DEFAULT_PILOT_USER_TASK)
    parser.add_argument("--injection-task", default=DEFAULT_PILOT_INJECTION_TASK)
    parser.add_argument("--attack", default=DEFAULT_PILOT_ATTACK, choices=["direct", "ignore_previous", "system_message", "injecagent"])
    parser.add_argument("--benchmark-version", default="v1.2.2")
    parser.add_argument("--provider", default="openai")
    parser.add_argument("--model", default="gpt-4o-mini")
    parser.add_argument("--mode", choices=["control", "governed"], default="governed")
    parser.add_argument("--kerna", default=shutil.which("kerna") or str(REPOSITORY_ROOT / "target" / "debug" / ("kerna.exe" if sys.platform == "win32" else "kerna")))
    parser.add_argument("--python", default=sys.executable)
    parser.add_argument("--deny-tool", action="append", default=[])
    parser.add_argument("--max-runtime-seconds", type=int, default=120)
    parser.add_argument("--max-tool-calls", type=int, default=12)
    parser.add_argument("--max-llm-calls", type=int, default=8)
    parser.add_argument("--max-cost-usd", type=float, default=0.10)
    parser.add_argument("--max-output-bytes", type=int, default=50_000)
    parser.add_argument("--output", type=Path, default=Path("reports/agentdojo"))
    parser.add_argument("--execute", action="store_true", help="Permit a real model call.")
    args = parser.parse_args()
    require_agentdojo()

    from agentdojo.attacks import baseline_attacks  # noqa: F401 - registers fixed attacks
    from agentdojo.attacks.attack_registry import load_attack
    from agentdojo.task_suite.load_suites import get_suite

    suite = get_suite(args.benchmark_version, args.suite)
    user_task = suite.get_user_task_by_id(args.user_task)
    injection_task = suite.get_injection_task_by_id(args.injection_task)
    attack = load_attack(args.attack, suite, AttackIdentity())
    injections = attack.attack(user_task, injection_task)
    deny_tools = list(dict.fromkeys(args.deny_tool))
    if args.mode == "governed" and args.suite == "workspace":
        deny_tools = list(dict.fromkeys([*WORKSPACE_MUTATING_TOOLS, *deny_tools]))
    scenario = {
        "benchmark_version": args.benchmark_version,
        "suite": args.suite,
        "user_task": args.user_task,
        "injection_task": args.injection_task,
        "attack": args.attack,
        "injections": injections,
        "mode": args.mode,
        "denied_tools": deny_tools,
    }

    run_root = args.output.resolve() / f"{args.suite}-{args.user_task}-{args.injection_task}-{args.attack}-{args.mode}"
    run_root.mkdir(parents=True, exist_ok=True)
    scenario_path = run_root / "scenario.json"
    bridge_result_path = run_root / "bridge-state.json"
    scenario_path.write_text(json.dumps(scenario, indent=2), encoding="utf-8")

    if not args.execute:
        print(json.dumps({"dryRun": True, "scenario": scenario, "runDirectory": str(run_root)}, indent=2))
        return 0

    if args.mode == "control":
        result = run_native_control(scenario, args)
        result.update(
            {
                "adapter": "agentdojo-native-control",
                "adapterVersion": REQUIRED_AGENTDOJO_VERSION,
                "attack": args.attack,
                "mode": args.mode,
                "deniedTools": [],
                "returnCode": 0,
                "kernaStdout": "",
                "kernaStderr": "",
            }
        )
        (run_root / "result.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
        print(json.dumps(result, indent=2))
        return 0

    if not Path(args.kerna).is_file():
        raise RuntimeError(f"Kerna executable not found: {args.kerna}")
    run_directory = Path(tempfile.mkdtemp(prefix="kerna-agentdojo-"))
    database_path = run_directory / "kerna.db"
    config_path = run_directory / "kerna.toml"
    write_kerna_config(
        config_path,
        args.python,
        scenario_path,
        bridge_result_path,
        database_path,
        run_directory / "sandbox",
        args,
        deny_tools,
    )
    execution = subprocess.run(
        [args.kerna, "run", user_task.PROMPT],
        cwd=run_directory,
        env=os.environ.copy(),
        text=True,
        encoding="utf-8",
        errors="replace",
        capture_output=True,
    )
    if not bridge_result_path.is_file():
        raise RuntimeError(f"Bridge did not produce state. Kerna stderr:\n{execution.stderr}")
    result = score_kerna_result(scenario, bridge_result_path, database_path)
    result.update(
        {
            "adapter": "kerna-agentdojo-mcp",
            "adapterVersion": "0.1.0",
            "attack": args.attack,
            "mode": args.mode,
            "deniedTools": deny_tools,
            "returnCode": execution.returncode,
            "kernaStdout": execution.stdout[-4000:],
            "kernaStderr": execution.stderr[-4000:],
        }
    )
    (run_root / "result.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
    print(json.dumps(result, indent=2))
    return 0 if execution.returncode == 0 else execution.returncode


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except RuntimeError as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(2)
