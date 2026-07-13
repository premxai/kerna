import sys
import json
import pyautogui
import os

def send_response(response):
    sys.stdout.write(json.dumps(response) + "\n")
    sys.stdout.flush()

def handle_request(req):
    method = req.get("method")
    params = req.get("params", {})
    req_id = req.get("id")

    if method == "get_tools":
        return {
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "tools": [
                    {
                        "name": "desktop_click",
                        "description": "Click at specific screen coordinates",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "x": {"type": "integer"},
                                "y": {"type": "integer"}
                            },
                            "required": ["x", "y"]
                        }
                    },
                    {
                        "name": "desktop_type",
                        "description": "Type text on the keyboard",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "text": {"type": "string"}
                            },
                            "required": ["text"]
                        }
                    },
                    {
                        "name": "fs_list_dir",
                        "description": "List files in a directory",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "path": {"type": "string"}
                            },
                            "required": ["path"]
                        }
                    }
                ]
            }
        }
    
    elif method == "call_tool":
        tool_name = params.get("name")
        tool_args = params.get("arguments", {})
        
        try:
            if tool_name == "desktop_click":
                x = tool_args.get("x")
                y = tool_args.get("y")
                pyautogui.click(x=x, y=y)
                result = f"Clicked at ({x}, {y})"
            
            elif tool_name == "desktop_type":
                text = tool_args.get("text")
                pyautogui.write(text, interval=0.01)
                result = f"Typed text: {text}"
            
            elif tool_name == "fs_list_dir":
                path = tool_args.get("path")
                files = os.listdir(path)
                result = "\n".join(files)
            
            else:
                raise ValueError(f"Unknown tool: {tool_name}")

            return {
                "jsonrpc": "2.0",
                "id": req_id,
                "result": {
                    "content": [{"type": "text", "text": result}]
                }
            }

        except Exception as e:
            return {
                "jsonrpc": "2.0",
                "id": req_id,
                "error": {
                    "code": -32603,
                    "message": str(e)
                }
            }

    else:
        return {
            "jsonrpc": "2.0",
            "id": req_id,
            "error": {
                "code": -32601,
                "message": "Method not found"
            }
        }

def main():
    # Make sure pyautogui doesn't abort
    pyautogui.FAILSAFE = False
    
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
            res = handle_request(req)
            if res:
                send_response(res)
        except Exception as e:
            send_response({
                "jsonrpc": "2.0",
                "id": None,
                "error": {"code": -32700, "message": f"Parse error: {e}"}
            })

if __name__ == "__main__":
    main()
