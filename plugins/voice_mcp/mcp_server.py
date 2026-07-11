#!/usr/bin/env python3
"""Kerna plugin: voice output/input (text-to-speech, speech-to-text).

Speaks standard MCP over stdio. Heavy deps (pyttsx3, speech_recognition) are
imported lazily only when a voice tool is actually called, so the server loads
and lists its tools even where those packages aren't installed.
"""
import sys
import json

TOOLS = [
    {"name": "voice_speak", "description": "Speak text aloud using text-to-speech.",
     "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]}},
    {"name": "voice_listen", "description": "Listen to the microphone and return transcribed text.",
     "inputSchema": {"type": "object", "properties": {"timeout_seconds": {"type": "integer", "default": 5}}}},
]


def call(name, args):
    if name == "voice_speak":
        import pyttsx3  # lazy
        engine = pyttsx3.init()
        engine.setProperty("rate", 150)
        engine.say(args.get("text", ""))
        engine.runAndWait()
        return "spoke: '%s'" % args.get("text", "")
    if name == "voice_listen":
        import speech_recognition as sr  # lazy
        recognizer = sr.Recognizer()
        with sr.Microphone() as source:
            recognizer.adjust_for_ambient_noise(source)
            audio = recognizer.listen(source, timeout=args.get("timeout_seconds", 5))
        try:
            return recognizer.recognize_google(audio)
        except sr.WaitTimeoutError:
            return "[no speech detected]"
    raise ValueError("unknown tool: %s" % name)


def handle(req):
    method = req.get("method")
    rid = req.get("id")
    if method == "initialize":
        return {"jsonrpc": "2.0", "id": rid, "result": {
            "protocolVersion": "2024-11-05", "capabilities": {"tools": {}},
            "serverInfo": {"name": "voice", "version": "1.0.0"}}}
    if method in ("notifications/initialized", "notifications/cancelled"):
        return None
    if method == "tools/list":
        return {"jsonrpc": "2.0", "id": rid, "result": {"tools": TOOLS}}
    if method == "tools/call":
        params = req.get("params", {})
        try:
            text = call(params.get("name"), params.get("arguments", {}))
            return {"jsonrpc": "2.0", "id": rid, "result": {"content": [{"type": "text", "text": text}]}}
        except Exception as e:  # noqa: BLE001
            return {"jsonrpc": "2.0", "id": rid, "result": {"isError": True, "content": [{"type": "text", "text": "error: %s" % e}]}}
    return {"jsonrpc": "2.0", "id": rid, "error": {"code": -32601, "message": "method not found"}}


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            resp = handle(json.loads(line))
        except Exception:  # noqa: BLE001
            resp = {"jsonrpc": "2.0", "id": None, "error": {"code": -32700, "message": "parse error"}}
        if resp is not None:
            sys.stdout.write(json.dumps(resp) + "\n")
            sys.stdout.flush()


if __name__ == "__main__":
    main()
