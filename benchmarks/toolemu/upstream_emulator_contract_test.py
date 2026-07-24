#!/usr/bin/env python3
"""Free contract: Kerna -> MCP bridge -> ToolEmu's upstream simulator path."""

from __future__ import annotations

import argparse
import json
import sqlite3
import tempfile
from datetime import UTC, datetime
from pathlib import Path

from gateway_adapter import GatewayClient, LoopbackBridgeServer, REPO_ROOT, ToolEmuCaseEnvironment, mcp_tool_name
from upstream_simulator import UpstreamToolEmuSimulator


def fake_tool_emulator_llm(response: str):
    """Create ToolEmu's own LLM type without making a network request."""
    from langchain.schema import AIMessage, ChatGeneration, ChatResult
    from pydantic import Field
    from toolemu.utils.llm import ChatOpenAI as ToolEmuChatOpenAI

    class FakeToolEmuChatOpenAI(ToolEmuChatOpenAI):
        responses: list[str] = Field(default_factory=list)
        response_index: int = 0

        def _generate(self, _messages, stop=None, run_manager=None, **_kwargs):
            del stop, run_manager
            text = self.responses[min(self.response_index, len(self.responses) - 1)]
            self.response_index += 1
            return ChatResult(
                generations=[ChatGeneration(message=AIMessage(content=text), generation_info={"finish_reason": "stop"})],
                # ToolEmu subclasses LangChain's legacy OpenAI wrapper, whose
                # result combiner unconditionally reads this field.
                llm_output={"model_name": self.model_name, "token_usage": {}},
            )

    return FakeToolEmuChatOpenAI(
        model_name="gpt-4-0613",
        openai_api_key="not-used-by-contract-test",
        responses=[response],
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--kerna", default="target/debug/kerna.exe")
    parser.add_argument("--assets", type=Path, default=REPO_ROOT / "reports" / "toolemu-source" / "assets")
    parser.add_argument("--out", default="reports/toolemu/upstream-emulator-contract.json")
    args = parser.parse_args()
    kerna = (REPO_ROOT / args.kerna).resolve()
    if not kerna.is_file():
        raise SystemExit(f"Kerna executable not found: {kerna}. Run `cargo build` first.")

    # This exact upstream parser expects both a simulator summary and an
    # observation. The fake response avoids any provider request.
    simulator = UpstreamToolEmuSimulator(
        args.assets.resolve(),
        "official_0",
        fake_tool_emulator_llm('Simulator Log Summary: Found no non-important matching tasks.\nObservation: {"tasks":[]}'),
    )
    direct_probe = simulator.observe(
        {"toolkit": "Todoist", "tool": "SearchTasks", "arguments": {"keywords": "Test", "max_results": 10}}
    )
    environment = ToolEmuCaseEnvironment.from_assets(args.assets.resolve(), "official_0", simulator.observe)
    search = mcp_tool_name("Todoist", "SearchTasks")
    report: dict[str, object] = {"benchmark": "ToolEmu upstream-emulator Kerna contract", "version": 1, "executedAt": datetime.now(UTC).isoformat(), "case": "official_0"}
    with tempfile.TemporaryDirectory(prefix="kerna-toolemu-upstream-contract-") as temporary:
        run_dir = Path(temporary)
        with LoopbackBridgeServer(environment) as bridge:
            gateway = GatewayClient.start(kerna=kerna, run_dir=run_dir, endpoint=bridge.endpoint, token=bridge.token, allowed_tools=[search])
            try:
                result = gateway.call_tool(search, {"keywords": "Test", "max_results": 10})
                blocked = gateway.call_tool(mcp_tool_name("Todoist", "DeleteTask"), {"task_id": "important-task"})
                task_id = gateway.task_id
            finally:
                gateway.close()
        connection = sqlite3.connect(run_dir / "kerna-gateway.db")
        events = [row[0] for row in connection.execute("select event_type from events where task_id = ? order by sequence", (task_id,))]
        connection.close()
    text = result.get("content", [{}])[0].get("text") if result.get("content") else None
    try:
        parsed_observation = json.loads(text) if isinstance(text, str) else None
    except json.JSONDecodeError:
        parsed_observation = None
    report.update({
        "status": "completed", "kernaTaskId": task_id, "directProbe": direct_probe, "toolEmuTrace": environment.trace,
        "returnedObservation": parsed_observation, "rawReturnedObservation": text,
        "unapprovedDeleteBlocked": bool(blocked.get("isError")), "receiptEvents": events,
    })
    report["passed"] = (
        report["directProbe"] == '{"tasks":[]}'
        and report["returnedObservation"] == {"tasks": []}
        and len(environment.trace) == 1
        and environment.trace[0]["tool"] == "SearchTasks"
        and report["unapprovedDeleteBlocked"]
        and events == [
            "tool.call.requested", "tool.policy.checked", "tool.call.completed",
            "tool.call.requested", "tool.policy.checked", "tool.call.blocked",
        ]
    )
    output = (REPO_ROOT / args.out).resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
