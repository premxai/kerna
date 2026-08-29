"""Black-box MCP acceptance for the Kerna filesystem fixture."""

import json
import os
import re
import subprocess

ROOT = os.path.dirname(os.path.abspath(__file__))
KERNA = os.environ.get("KERNA_BIN", "kerna")


def request(proc, request_id, method, params=None):
    message = {"jsonrpc": "2.0", "id": request_id, "method": method}
    if params is not None:
        message["params"] = params
    proc.stdin.write(json.dumps(message) + "\n")
    proc.stdin.flush()
    while True:
        line = proc.stdout.readline()
        if not line:
            raise RuntimeError("gateway exited: " + proc.stderr.read())
        response = json.loads(line)
        if response.get("id") == request_id:
            return response


def notify(proc, method):
    proc.stdin.write(json.dumps({"jsonrpc": "2.0", "method": method}) + "\n")
    proc.stdin.flush()


def main():
    proc = subprocess.Popen(
        [KERNA, "gateway", "--workspace", ROOT],
        cwd=ROOT,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    output = os.path.join(ROOT, "write", "blackbox.txt")
    try:
        initialized = request(proc, 1, "initialize", {"protocolVersion": "2025-06-18"})
        assert initialized["result"]["protocolVersion"] == "2025-06-18"
        notify(proc, "notifications/initialized")
        names = {tool["name"] for tool in request(proc, 2, "tools/list")["result"]["tools"]}
        assert {"read_file", "write_file", "network_probe", "kerna_session_status"} <= names
        read = request(proc, 3, "tools/call", {"name": "read_file", "arguments": {"path": "hello.txt"}})
        assert "real project file" in read["result"]["content"][0]["text"]
        write = {"name": "write_file", "arguments": {"path": "blackbox.txt", "content": "black-box approval"}}
        blocked = request(proc, 4, "tools/call", write)
        assert blocked["result"]["isError"] and "approval approve" in blocked["result"]["content"][0]["text"]
        pending = subprocess.run([KERNA, "approval", "list"], cwd=ROOT, capture_output=True, text=True, check=True).stdout
        approval = re.search(r"[0-9a-f]{8}-[0-9a-f-]{27,}", pending).group(0)
        subprocess.run([KERNA, "approval", "approve", approval], cwd=ROOT, check=True, capture_output=True, text=True)
        assert request(proc, 5, "tools/call", write)["result"]["content"][0]["text"] == "written"
        assert open(output, encoding="utf-8").read() == "black-box approval"
        network = request(proc, 6, "tools/call", {"name": "network_probe", "arguments": {}})
        assert "network unavailable" in network["result"]["content"][0]["text"]
        card = request(proc, 7, "tools/call", {"name": "kerna_session_status", "arguments": {}})
        card_text = card["result"]["content"][0]["text"]
        assert all(label in card_text for label in ("Tools exposed:", "Write state:", "Last decision:", "Trace ID:"))
        print("black-box MCP gateway acceptance: PASS")
    finally:
        if os.path.exists(output):
            os.remove(output)
        proc.terminate()
        proc.wait(timeout=5)


if __name__ == "__main__":
    main()
