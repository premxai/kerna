#!/usr/bin/env python3
"""Google Calendar MCP connector for Kerna.

This plugin deliberately contains the Google-specific behavior; Kerna only
spawns and governs it. It uses an OAuth refresh token injected by Kerna, never
persists access tokens, and uses only the Python standard library.

Run ``python connect.py`` once to grant Calendar access. The default consent
scope is read-only. Re-run it with ``--allow-write`` only if you need the
explicitly approval-gated ``google_create_event`` tool.
"""
import json
import os
import sys
import urllib.error
import urllib.parse
import urllib.request
from datetime import datetime, timedelta, timezone

TOKEN_URL = "https://oauth2.googleapis.com/token"
API_ROOT = "https://www.googleapis.com/calendar/v3"
TIMEOUT_SECONDS = 20


def _required_env(name):
    value = os.environ.get(name, "").strip()
    if not value:
        raise ValueError(
            "%s is not set. Run `python plugins/google_calendar_mcp/connect.py` "
            "and restart Kerna." % name
        )
    return value


def _json_request(url, method="GET", data=None, headers=None):
    payload = None if data is None else json.dumps(data).encode("utf-8")
    request_headers = {"Accept": "application/json"}
    if payload is not None:
        request_headers["Content-Type"] = "application/json"
    request_headers.update(headers or {})
    request = urllib.request.Request(url, data=payload, headers=request_headers, method=method)
    try:
        with urllib.request.urlopen(request, timeout=TIMEOUT_SECONDS) as response:
            return json.loads(response.read().decode("utf-8"))
    except urllib.error.HTTPError as exc:
        # Provider messages can contain user data. Keep the error category and
        # status, but never mirror a response body into a task receipt.
        if exc.code in (401, 403):
            raise ValueError("Google Calendar authorization failed (%s); reconnect may be required" % exc.code)
        raise ValueError("Google Calendar request failed (%s)" % exc.code)
    except urllib.error.URLError:
        raise ValueError("Google Calendar is unreachable; check your connection")


def _access_token():
    form = urllib.parse.urlencode(
        {
            "client_id": _required_env("KERNA_GOOGLE_CALENDAR_CLIENT_ID"),
            "refresh_token": _required_env("KERNA_GOOGLE_CALENDAR_REFRESH_TOKEN"),
            "grant_type": "refresh_token",
        }
    ).encode("utf-8")
    request = urllib.request.Request(
        TOKEN_URL,
        data=form,
        headers={"Content-Type": "application/x-www-form-urlencoded", "Accept": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=TIMEOUT_SECONDS) as response:
            result = json.loads(response.read().decode("utf-8"))
    except urllib.error.HTTPError as exc:
        if exc.code in (400, 401):
            raise ValueError("Google Calendar authorization expired or was revoked; reconnect is required")
        raise ValueError("Google OAuth token refresh failed (%s)" % exc.code)
    except urllib.error.URLError:
        raise ValueError("Google OAuth is unreachable; check your connection")
    token = result.get("access_token")
    if not isinstance(token, str) or not token:
        raise ValueError("Google OAuth returned no usable access token")
    return token


def _calendar_url(calendar_id, suffix=""):
    encoded = urllib.parse.quote(str(calendar_id or "primary"), safe="")
    return "%s/calendars/%s%s" % (API_ROOT, encoded, suffix)


def _rfc3339(value, field):
    if not isinstance(value, str) or not value.strip():
        raise ValueError("%s must be a non-empty RFC3339 date-time" % field)
    normalized = value.strip().replace("Z", "+00:00")
    try:
        parsed = datetime.fromisoformat(normalized)
    except ValueError:
        raise ValueError("%s must be RFC3339, for example 2026-07-15T09:00:00-04:00" % field)
    if parsed.tzinfo is None:
        raise ValueError("%s must include a timezone offset" % field)
    return parsed.isoformat()


def _event_time(event, key):
    value = event.get(key, {})
    return value.get("dateTime") or value.get("date") or "unscheduled"


def google_calendar_status(calendar_id="primary"):
    calendar = _json_request(
        _calendar_url(calendar_id), headers={"Authorization": "Bearer %s" % _access_token()}
    )
    return "Connected to Google Calendar: %s" % calendar.get("summary", calendar_id)


def google_list_events(calendar_id="primary", time_min=None, time_max=None, max_results=20):
    now = datetime.now(timezone.utc)
    start = _rfc3339(time_min, "time_min") if time_min else now.isoformat()
    end = _rfc3339(time_max, "time_max") if time_max else (now + timedelta(days=30)).isoformat()
    try:
        limit = max(1, min(int(max_results), 100))
    except (TypeError, ValueError):
        raise ValueError("max_results must be a number between 1 and 100")
    query = urllib.parse.urlencode(
        {"timeMin": start, "timeMax": end, "singleEvents": "true", "orderBy": "startTime", "maxResults": limit}
    )
    result = _json_request(
        _calendar_url(calendar_id, "/events?" + query),
        headers={"Authorization": "Bearer %s" % _access_token()},
    )
    events = result.get("items", [])
    if not events:
        return "No Google Calendar events in the selected window."
    lines = []
    for event in events:
        summary = str(event.get("summary") or "(untitled)").replace("\n", " ")
        location = str(event.get("location") or "").replace("\n", " ")
        detail = "%s — %s" % (_event_time(event, "start"), summary)
        if location:
            detail += " @ " + location
        lines.append(detail)
    return "\n".join(lines)


def google_create_event(summary, start, end, calendar_id="primary", location=None, description=None, time_zone=None, send_updates="none"):
    if send_updates not in ("none", "all", "externalOnly"):
        raise ValueError("send_updates must be none, all, or externalOnly")
    start_value = _rfc3339(start, "start")
    end_value = _rfc3339(end, "end")
    if datetime.fromisoformat(end_value) <= datetime.fromisoformat(start_value):
        raise ValueError("end must be after start")
    body = {
        "summary": str(summary).strip(),
        "start": {"dateTime": start_value},
        "end": {"dateTime": end_value},
    }
    if not body["summary"]:
        raise ValueError("summary must not be empty")
    if location:
        body["location"] = str(location)
    if description:
        body["description"] = str(description)
    if time_zone:
        body["start"]["timeZone"] = str(time_zone)
        body["end"]["timeZone"] = str(time_zone)
    result = _json_request(
        _calendar_url(calendar_id, "/events?" + urllib.parse.urlencode({"sendUpdates": send_updates})),
        method="POST",
        data=body,
        headers={"Authorization": "Bearer %s" % _access_token()},
    )
    created_summary = str(result.get("summary", body["summary"])).replace("\n", " ")
    return "Created Google Calendar event: %s (%s)" % (created_summary, _event_time(result, "start"))


TOOLS = [
    {
        "name": "google_calendar_status",
        "description": "Check whether the configured Google Calendar connection is usable. It does not change calendar data.",
        "inputSchema": {"type": "object", "properties": {"calendar_id": {"type": "string", "default": "primary"}}},
    },
    {
        "name": "google_list_events",
        "description": "List Google Calendar events in a bounded date window. This is read-only.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "calendar_id": {"type": "string", "default": "primary"},
                "time_min": {"type": "string", "description": "RFC3339 start; defaults to now."},
                "time_max": {"type": "string", "description": "RFC3339 end; defaults to 30 days from now."},
                "max_results": {"type": "integer", "default": 20, "minimum": 1, "maximum": 100},
            },
        },
    },
    {
        "name": "google_create_event",
        "description": "Create one Google Calendar event. Requires explicit Kerna approval and a write-enabled OAuth grant; notifications default to none.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "summary": {"type": "string"},
                "start": {"type": "string", "description": "RFC3339 with timezone"},
                "end": {"type": "string", "description": "RFC3339 with timezone"},
                "calendar_id": {"type": "string", "default": "primary"},
                "location": {"type": "string"},
                "description": {"type": "string"},
                "time_zone": {"type": "string"},
                "send_updates": {"type": "string", "enum": ["none", "all", "externalOnly"], "default": "none"},
            },
            "required": ["summary", "start", "end"],
        },
    },
]


def call(name, args):
    if name == "google_calendar_status":
        return google_calendar_status(args.get("calendar_id", "primary"))
    if name == "google_list_events":
        return google_list_events(
            args.get("calendar_id", "primary"), args.get("time_min"), args.get("time_max"), args.get("max_results", 20)
        )
    if name == "google_create_event":
        return google_create_event(
            args["summary"], args["start"], args["end"], args.get("calendar_id", "primary"), args.get("location"),
            args.get("description"), args.get("time_zone"), args.get("send_updates", "none"),
        )
    raise ValueError("unknown tool: %s" % name)


def handle(request):
    method, request_id = request.get("method"), request.get("id")
    if method == "initialize":
        return {"jsonrpc": "2.0", "id": request_id, "result": {"protocolVersion": "2024-11-05", "capabilities": {"tools": {}}, "serverInfo": {"name": "google-calendar", "version": "0.1.0"}}}
    if method in ("notifications/initialized", "notifications/cancelled"):
        return None
    if method == "tools/list":
        return {"jsonrpc": "2.0", "id": request_id, "result": {"tools": TOOLS}}
    if method == "tools/call":
        params = request.get("params", {})
        try:
            text = call(params.get("name"), params.get("arguments", {}))
            return {"jsonrpc": "2.0", "id": request_id, "result": {"content": [{"type": "text", "text": text}]}}
        except Exception as exc:  # a plugin must return MCP errors, not crash the stdio stream
            return {"jsonrpc": "2.0", "id": request_id, "result": {"isError": True, "content": [{"type": "text", "text": "error: %s" % exc}]}}
    return {"jsonrpc": "2.0", "id": request_id, "error": {"code": -32601, "message": "method not found"}}


def main():
    for line in sys.stdin:
        try:
            request = json.loads(line)
            response = handle(request)
        except Exception:
            response = {"jsonrpc": "2.0", "id": None, "error": {"code": -32700, "message": "parse error"}}
        if response is not None:
            sys.stdout.write(json.dumps(response) + "\n")
            sys.stdout.flush()


if __name__ == "__main__":
    main()
