import sys
import json
import pyttsx3
import speech_recognition as sr

# Initialize TTS Engine (Fallback for Piper)
tts_engine = pyttsx3.init()
tts_engine.setProperty('rate', 150)

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
                        "name": "voice_speak",
                        "description": "Speak text aloud to the user using TTS",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "text": {"type": "string"}
                            },
                            "required": ["text"]
                        }
                    },
                    {
                        "name": "voice_listen",
                        "description": "Listen to the user's microphone and return transcribed text",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "timeout_seconds": {"type": "integer", "default": 5}
                            }
                        }
                    }
                ]
            }
        }
    
    elif method == "call_tool":
        tool_name = params.get("name")
        tool_args = params.get("arguments", {})
        
        try:
            if tool_name == "voice_speak":
                text = tool_args.get("text")
                # Blocking call to speak
                tts_engine.say(text)
                tts_engine.runAndWait()
                result = f"Spoke text: '{text}'"
                
            elif tool_name == "voice_listen":
                timeout = tool_args.get("timeout_seconds", 5)
                recognizer = sr.Recognizer()
                with sr.Microphone() as source:
                    recognizer.adjust_for_ambient_noise(source)
                    audio = recognizer.listen(source, timeout=timeout)
                    
                # Use Google's free API for quick prototyping, or fallback to faster-whisper if configured
                text = recognizer.recognize_google(audio)
                result = text
                
            else:
                raise ValueError(f"Unknown tool: {tool_name}")

            return {
                "jsonrpc": "2.0",
                "id": req_id,
                "result": {
                    "content": [{"type": "text", "text": result}]
                }
            }

        except sr.WaitTimeoutError:
            return {
                "jsonrpc": "2.0",
                "id": req_id,
                "result": {
                    "content": [{"type": "text", "text": "[No speech detected]"}]
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
