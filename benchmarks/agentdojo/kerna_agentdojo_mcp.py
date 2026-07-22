#!/usr/bin/env python3
"""Task-scoped MCP server that exposes an AgentDojo task environment to Kerna.

This plugin belongs to the benchmark harness, not the Kerna kernel. It turns
the functions registered by one AgentDojo task suite into stdio MCP tools and
writes a redacted state/trace snapshot after every executed call.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any


def json_safe(value: Any) -> Any:
    if hasattr(value, "model_dump"):
        return value.model_dump(mode="json")
    if isinstance(value, list | tuple):
        return [json_safe(item) for item in value]
    if isinstance(value, dict):
        return {str(key): json_safe(item) for key, item in value.items()}
    return value


class AgentDojoBridge:
    def __init__(self, scenario_path: Path, result_path: Path) -> None:
        from agentdojo.functions_runtime import FunctionCall, FunctionsRuntime
        from agentdojo.task_suite.load_suites import get_suite

        self._function_call_type = FunctionCall
        self._scenario = json.loads(scenario_path.read_text(encoding="utf-8"))
        self._result_path = result_path
        self._suite = get_suite(
            self._scenario.get("benchmark_version", "v1.2.2"), self._scenario["suite"]
        )
        self._user_task = self._suite.get_user_task_by_id(self._scenario["user_task"])
        injection_task_id = self._scenario.get("injection_task")
        self._injection_task = (
            self._suite.get_injection_task_by_id(injection_task_id) if injection_task_id else None
        )
        environment = self._suite.load_and_inject_default_environment(
            self._scenario.get("injections", {})
        )
        self._environment = self._user_task.init_environment(environment)
        self._pre_environment = self._environment.model_copy(deep=True)
        self._runtime = FunctionsRuntime(self._suite.tools)
        self._trace: list[Any] = []
        self.write_snapshot()

    def tools(self) -> list[dict[str, Any]]:
        return [
            {
                "name": function.name,
                "description": function.description,
                "inputSchema": function.parameters.model_json_schema(),
            }
            for function in self._suite.tools
        ]

    def call(self, name: str, arguments: dict[str, Any]) -> dict[str, Any]:
        result, error = self._runtime.run_function(self._environment, name, arguments)
        self._trace.append(self._function_call_type(function=name, args=dict(arguments)))
        self.write_snapshot()
        if error:
            return {
                "isError": True,
                "content": [{"type": "text", "text": error}],
            }
        return {
            "content": [
                {
                    "type": "text",
                    "text": json.dumps(json_safe(result), ensure_ascii=False),
                }
            ]
        }

    def write_snapshot(self) -> None:
        payload = {
            "schemaVersion": 1,
            "suite": self._suite.name,
            "userTask": self._user_task.ID,
            "injectionTask": self._injection_task.ID if self._injection_task else None,
            "preEnvironment": json_safe(self._pre_environment),
            "postEnvironment": json_safe(self._environment),
            "functionTrace": [json_safe(call) for call in self._trace],
        }
        self._result_path.parent.mkdir(parents=True, exist_ok=True)
        temporary = self._result_path.with_suffix(".tmp")
        temporary.write_text(json.dumps(payload, indent=2), encoding="utf-8")
        temporary.replace(self._result_path)


def response(request_id: Any, result: dict[str, Any]) -> dict[str, Any]:
    return {"jsonrpc": "2.0", "id": request_id, "result": result}


def error(request_id: Any, code: int, message: str) -> dict[str, Any]:
    return {
        "jsonrpc": "2.0",
        "id": request_id,
        "error": {"code": code, "message": message},
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--scenario", type=Path, required=True)
    parser.add_argument("--result", type=Path, required=True)
    args = parser.parse_args()
    bridge = AgentDojoBridge(args.scenario.resolve(), args.result.resolve())

    for line in sys.stdin:
        try:
            request = json.loads(line)
            request_id = request.get("id")
            method = request.get("method")
            if method == "notifications/initialized":
                continue
            if method == "initialize":
                payload = response(
                    request_id,
                    {
                        "protocolVersion": "2024-11-05",
                        "capabilities": {"tools": {}},
                        "serverInfo": {"name": "kerna-agentdojo-bridge", "version": "0.1.0"},
                    },
                )
            elif method == "tools/list":
                payload = response(request_id, {"tools": bridge.tools()})
            elif method == "tools/call":
                params = request.get("params", {})
                payload = response(
                    request_id,
                    bridge.call(params.get("name", ""), params.get("arguments", {})),
                )
            else:
                payload = error(request_id, -32601, f"Method not found: {method}")
        except Exception as exc:  # Keep protocol failures inside JSON-RPC.
            payload = error(request.get("id") if "request" in locals() else None, -32000, str(exc))
        print(json.dumps(payload), flush=True)
    bridge.write_snapshot()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
