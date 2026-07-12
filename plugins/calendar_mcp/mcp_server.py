#!/usr/bin/env python3
"""Kerna starter plugin: local calendar (iCalendar / .ics).

Speaks standard MCP over stdio. Pure Python standard library. Reads and appends
events to a single local .ics file (KERNA_CALENDAR_FILE, default
./calendar.ics) — no external account, nothing leaves your machine. A minimal
VEVENT reader/writer; not a full RFC-5545 implementation, but enough for
day-to-day "what's on today" and "add an event".
"""
import os
import sys
import json
import uuid
from datetime import datetime

DATEFMT = "%Y%m%dT%H%M%S"


def _cal_file():
    return os.environ.get("KERNA_CALENDAR_FILE", "calendar.ics").strip() or "calendar.ics"


def _parse_events(text):
    events = []
    cur = None
    for raw in text.splitlines():
        line = raw.strip()
        if line == "BEGIN:VEVENT":
            cur = {}
        elif line == "END:VEVENT":
            if cur is not None:
                events.append(cur)
            cur = None
        elif cur is not None and ":" in line:
            key, val = line.split(":", 1)
            key = key.split(";", 1)[0]  # drop params like ;VALUE=DATE
            cur[key] = val
    return events


def _fmt_dt(v):
    for fmt in (DATEFMT, "%Y%m%d", "%Y%m%dT%H%M%SZ"):
        try:
            return datetime.strptime(v, fmt).strftime("%Y-%m-%d %H:%M")
        except ValueError:
            continue
    return v


def list_events(limit=20):
    path = _cal_file()
    if not os.path.exists(path):
        return "(no calendar yet — add an event to create %s)" % path
    with open(path, "r", encoding="utf-8", errors="replace") as f:
        events = _parse_events(f.read())
    events.sort(key=lambda e: e.get("DTSTART", ""))
    lines = []
    for e in events[:limit]:
        lines.append(
            "%s  %s%s"
            % (
                _fmt_dt(e.get("DTSTART", "?")),
                e.get("SUMMARY", "(untitled)"),
                (" @ " + e["LOCATION"]) if e.get("LOCATION") else "",
            )
        )
    return "\n".join(lines) if lines else "(no events)"


def add_event(summary, start, end=None, location=None):
    # start/end accepted as "YYYY-MM-DD HH:MM" or "YYYY-MM-DD".
    def _to_ics(s):
        s = s.strip()
        for fmt in ("%Y-%m-%d %H:%M", "%Y-%m-%d"):
            try:
                return datetime.strptime(s, fmt).strftime(DATEFMT)
            except ValueError:
                continue
        raise ValueError("bad date '%s' (use YYYY-MM-DD HH:MM)" % s)

    dtstart = _to_ics(start)
    dtend = _to_ics(end) if end else dtstart
    path = _cal_file()
    new = False
    if not os.path.exists(path):
        new = True
    with open(path, "a", encoding="utf-8") as f:
        if new:
            f.write("BEGIN:VCALENDAR\nVERSION:2.0\nPRODID:-//Kerna//calendar//EN\n")
        f.write("BEGIN:VEVENT\n")
        f.write("UID:%s\n" % uuid.uuid4())
        f.write("DTSTAMP:%s\n" % datetime.utcnow().strftime(DATEFMT))
        f.write("DTSTART:%s\n" % dtstart)
        f.write("DTEND:%s\n" % dtend)
        f.write("SUMMARY:%s\n" % summary.replace("\n", " "))
        if location:
            f.write("LOCATION:%s\n" % location.replace("\n", " "))
        f.write("END:VEVENT\n")
    return "Added '%s' at %s" % (summary, start)


TOOLS = [
    {
        "name": "list_events",
        "description": "List upcoming events from the local calendar, earliest first.",
        "inputSchema": {
            "type": "object",
            "properties": {"limit": {"type": "integer", "default": 20}},
        },
    },
    {
        "name": "add_event",
        "description": "Add an event to the local calendar. Dates like 'YYYY-MM-DD HH:MM'.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "summary": {"type": "string"},
                "start": {"type": "string"},
                "end": {"type": "string"},
                "location": {"type": "string"},
            },
            "required": ["summary", "start"],
        },
    },
]


def call(name, args):
    if name == "list_events":
        return list_events(int(args.get("limit", 20)))
    if name == "add_event":
        return add_event(
            args["summary"], args["start"], args.get("end"), args.get("location")
        )
    raise ValueError("unknown tool: %s" % name)


def handle(req):
    method = req.get("method")
    rid = req.get("id")
    if method == "initialize":
        return {"jsonrpc": "2.0", "id": rid, "result": {
            "protocolVersion": "2024-11-05", "capabilities": {"tools": {}},
            "serverInfo": {"name": "calendar", "version": "1.0.0"}}}
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
