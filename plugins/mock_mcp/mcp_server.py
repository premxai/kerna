import sys
import json
import time

def send_response(response):
    print(json.dumps(response), flush=True)

def send_error(message, req_id=None):
    print(json.dumps({"jsonrpc": "2.0", "id": req_id, "error": {"code": -32603, "message": message}}), flush=True)

def handle_request(req):
    req_id = req.get("id")
    method = req.get("method")
    
    if method == "initialize":
        send_response({
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "serverInfo": {"name": "MockMCP", "version": "1.0.0"},
                "capabilities": {"tools": {}}
            }
        })
        return

    if method == "tools/list":
        send_response({
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "tools": [{
                    "name": "run_mock_action",
                    "description": "Mock tool for testing Kerna runtime reliability.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "action": {
                                "type": "string",
                                "enum": ["echo", "sleep", "large_output", "error", "invalid_json", "exit", "permission_denied"],
                                "description": "The action to perform."
                            }
                        },
                        "required": ["action"]
                    }
                }]
            }
        })
        return
        
    if method == "tools/call":
        params = req.get("params", {})
        args = params.get("arguments", {})
        action = args.get("action")

        if action == "echo":
            send_response({"jsonrpc": "2.0", "id": req_id, "result": {"content": [{"type": "text", "text": "hello"}]}})
        elif action == "sleep":
            time.sleep(60)
            send_response({"jsonrpc": "2.0", "id": req_id, "result": {"content": [{"type": "text", "text": "woke up"}]}})
        elif action == "large_output":
            # 20MB of text
            large_text = "A" * (20 * 1024 * 1024)
            send_response({"jsonrpc": "2.0", "id": req_id, "result": {"content": [{"type": "text", "text": large_text}]}})
        elif action == "error":
            send_response({"jsonrpc": "2.0", "id": req_id, "result": {"content": [{"type": "text", "text": "Simulated error"}], "isError": True}})
        elif action == "invalid_json":
            print("this is not valid json { [ }", flush=True)
        elif action == "exit":
            sys.exit(1)
        elif action == "permission_denied":
            # This is technically handled by Kerna's config, but if it reaches here we simulate a failure
            send_error("Permission denied to execute this action.", req_id)
        else:
            send_error(f"Unknown action: {action}", req_id)
        return
        
    send_error(f"Method not found: {method}", req_id)

def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
            handle_request(req)
        except json.JSONDecodeError:
            send_error("Parse error")
        except Exception as e:
            send_error(str(e))

if __name__ == "__main__":
    main()
