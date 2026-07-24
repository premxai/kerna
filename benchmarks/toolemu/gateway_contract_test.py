#!/usr/bin/env python3
"""Free ToolEmu schema -> Kerna gateway -> same callback contract test."""

from __future__ import annotations

import argparse
import json
import sqlite3
import tempfile
from datetime import UTC, datetime
from pathlib import Path

from gateway_adapter import GatewayClient, LoopbackBridgeServer, REPO_ROOT, ToolEmuCaseEnvironment, mcp_tool_name


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--kerna", default="target/debug/kerna.exe")
    parser.add_argument("--assets", type=Path, default=REPO_ROOT / "reports" / "toolemu-source" / "assets")
    parser.add_argument("--out", default="reports/toolemu/gateway-contract.json")
    args = parser.parse_args()
    kerna = (REPO_ROOT / args.kerna).resolve()
    if not kerna.is_file():
        raise SystemExit(f"Kerna executable not found: {kerna}. Run `cargo build` first.")

    calls: list[dict[str, object]] = []

    def observe(request: dict[str, object]) -> dict[str, object]:
        calls.append({"toolkit": request["toolkit"], "tool": request["tool"], "arguments": request["arguments"]})
        if request["toolkit"] == "Todoist" and request["tool"] == "SearchTasks":
            return {"tasks": []}
        raise RuntimeError("contract simulator received an unexpected ToolEmu tool")

    environment = ToolEmuCaseEnvironment.from_assets(args.assets.resolve(), "official_0", observe)
    search = mcp_tool_name("Todoist", "SearchTasks")
    report: dict[str, object] = {"benchmark": "ToolEmu Kerna gateway adapter contract", "version": 1, "executedAt": datetime.now(UTC).isoformat(), "case": "official_0"}
    with tempfile.TemporaryDirectory(prefix="kerna-toolemu-contract-") as temporary:
        run_dir = Path(temporary)
        with LoopbackBridgeServer(environment) as bridge:
            gateway = GatewayClient.start(kerna=kerna, run_dir=run_dir, endpoint=bridge.endpoint, token=bridge.token, allowed_tools=[search])
            try:
                exposed = gateway.list_tools()
                result = gateway.call_tool(search, {"keywords": "Test", "max_results": 10})
                blocked = gateway.call_tool(mcp_tool_name("Todoist", "DeleteTask"), {"task_id": "important-task"})
                task_id = gateway.task_id
            finally:
                gateway.close()
        connection = sqlite3.connect(run_dir / "kerna-gateway.db")
        events = [row[0] for row in connection.execute("select event_type from events where task_id = ? order by sequence", (task_id,))]
        connection.close()
    result_text = result.get("content", [{}])[0].get("text") if result.get("content") else None
    report.update({
        "status": "completed", "kernaTaskId": task_id, "toolsExposed": [tool.get("name") for tool in exposed],
        "emulatorCalls": calls, "returnedObservation": json.loads(result_text) if isinstance(result_text, str) else None,
        "unapprovedDeleteBlocked": bool(blocked.get("isError")), "receiptEvents": events,
    })
    report["passed"] = (
        report["toolsExposed"] == [tool["name"] for tool in environment.tools()]
        and calls == [{"toolkit": "Todoist", "tool": "SearchTasks", "arguments": {"keywords": "Test", "max_results": 10}}]
        and report["returnedObservation"] == {"tasks": []}
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
