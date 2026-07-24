#!/usr/bin/env python3
"""Free integration contract: local tool -> Kerna gateway -> same local tool."""

from __future__ import annotations

import argparse
import json
import sqlite3
import tempfile
from datetime import datetime, timezone
from pathlib import Path

from gateway_adapter import GatewayClient, LoopbackBridgeServer, REPO_ROOT


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--kerna", default="target/debug/kerna.exe", help="Kerna executable relative to the repository")
    parser.add_argument("--out", default="reports/tau3/gateway-contract.json", help="JSON report path relative to the repository")
    args = parser.parse_args()
    kerna = (REPO_ROOT / args.kerna).resolve()
    if not kerna.is_file():
        raise SystemExit(f"Kerna executable not found: {kerna}. Run `cargo build` from the repository root first.")

    calls: list[dict[str, object]] = []
    schema = {
        "name": "echo_tau3_state",
        "description": "Contract-only loopback test tool.",
        "inputSchema": {"type": "object", "properties": {"value": {"type": "string"}}, "required": ["value"]},
    }

    def call_tool(name: str, arguments: dict[str, object]) -> dict[str, object]:
        calls.append({"name": name, "arguments": arguments})
        if name != "echo_tau3_state":
            return {"id": "contract", "role": "tool", "content": "unknown", "requestor": "assistant", "error": True}
        return {"id": "contract", "role": "tool", "content": arguments["value"], "requestor": "assistant", "error": False}

    report: dict[str, object] = {"benchmark": "tau3 Kerna gateway adapter contract", "version": 1, "executedAt": datetime.now(timezone.utc).isoformat()}
    with tempfile.TemporaryDirectory(prefix="kerna-tau3-contract-") as directory:
        run_dir = Path(directory)
        with LoopbackBridgeServer(lambda: [schema], call_tool) as bridge:
            gateway = GatewayClient.start(kerna=kerna, run_dir=run_dir, endpoint=bridge.endpoint, token=bridge.token, allowed_tools=["echo_tau3_state"])
            try:
                exposed = gateway.list_tools()
                result = gateway.call_tool("echo_tau3_state", {"value": "same-state"})
                blocked = gateway.call_tool("not_registered", {})
                task_id = gateway.task_id
            finally:
                gateway.close()
        events: list[str] = []
        if task_id:
            connection = sqlite3.connect(run_dir / "kerna-gateway.db")
            events = [row[0] for row in connection.execute("select event_type from events where task_id = ? order by sequence", (task_id,))]
            connection.close()
        result_text = result.get("content", [{}])[0].get("text") if result.get("content") else None
        report.update(
            {
                "status": "completed",
                "kernaTaskId": task_id,
                "toolsExposed": [tool.get("name") for tool in exposed],
                "loopbackCalls": calls,
                "returnedToolMessage": json.loads(result_text) if isinstance(result_text, str) else None,
                "unknownToolBlocked": bool(blocked.get("isError")),
                "receiptEvents": events,
                "passed": (
                    [tool.get("name") for tool in exposed] == ["echo_tau3_state"]
                    and calls == [{"name": "echo_tau3_state", "arguments": {"value": "same-state"}}]
                    and json.loads(result_text or "{}").get("content") == "same-state"
                    and bool(blocked.get("isError"))
                    and events == ["tool.call.requested", "tool.policy.checked", "tool.call.completed", "tool.call.blocked"]
                ),
            }
        )
    output = (REPO_ROOT / args.out).resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
