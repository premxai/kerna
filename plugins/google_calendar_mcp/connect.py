#!/usr/bin/env python3
"""Interactive OAuth setup for the Kerna Google Calendar MCP connector.

This is intentionally a connector-owned helper, not Kerna runtime logic. It
uses OAuth authorization-code + PKCE against a loopback listener on desktop
platforms. By default it asks only for calendar.events.readonly.

On Windows ``--save`` stores the refresh token in the current user's
environment, consistent with Kerna's existing environment-secret model. It is
never written to kerna.toml or printed unless ``--print-refresh-token`` is
explicitly requested for a non-Windows environment.
"""
import argparse
import base64
import hashlib
import json
import os
import secrets
import sys
import threading
import urllib.error
import urllib.parse
import urllib.request
import webbrowser
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

AUTHORIZE_URL = "https://accounts.google.com/o/oauth2/v2/auth"
TOKEN_URL = "https://oauth2.googleapis.com/token"
READ_SCOPE = "https://www.googleapis.com/auth/calendar.events.readonly"
WRITE_SCOPE = "https://www.googleapis.com/auth/calendar.events"


def _code_challenge(verifier):
    return base64.urlsafe_b64encode(hashlib.sha256(verifier.encode("ascii")).digest()).decode("ascii").rstrip("=")


def _save_windows_user_env(name, value):
    if os.name != "nt":
        return False
    import ctypes
    import winreg

    with winreg.CreateKey(winreg.HKEY_CURRENT_USER, r"Environment") as key:
        winreg.SetValueEx(key, name, 0, winreg.REG_SZ, value)
    # Tell already-running shells and desktop apps that environment settings
    # changed. They may still need a restart to inherit the new value.
    result = ctypes.c_ulong()
    ctypes.windll.user32.SendMessageTimeoutW(0xFFFF, 0x001A, 0, "Environment", 0x0002, 5000, ctypes.byref(result))
    return True


def _token_exchange(client_id, redirect_uri, verifier, code):
    body = urllib.parse.urlencode(
        {
            "client_id": client_id,
            "code": code,
            "code_verifier": verifier,
            "grant_type": "authorization_code",
            "redirect_uri": redirect_uri,
        }
    ).encode("utf-8")
    request = urllib.request.Request(
        TOKEN_URL,
        data=body,
        headers={"Content-Type": "application/x-www-form-urlencoded", "Accept": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            result = json.loads(response.read().decode("utf-8"))
    except urllib.error.HTTPError as exc:
        raise RuntimeError("Google rejected the OAuth exchange (%s). Check the desktop client ID and redirect configuration." % exc.code)
    except urllib.error.URLError:
        raise RuntimeError("Could not reach Google OAuth. Check your internet connection.")
    token = result.get("refresh_token")
    if not isinstance(token, str) or not token:
        raise RuntimeError("Google did not return a refresh token. Remove the previous grant, then retry so consent is shown again.")
    return token


def connect(client_id, allow_write, save, print_refresh_token):
    if save and os.name != "nt":
        raise RuntimeError("--save is currently supported on Windows only. Use --print-refresh-token in a secure shell on this platform.")
    if not save and not print_refresh_token:
        raise RuntimeError("Choose --save on Windows or --print-refresh-token for intentional secure-shell setup before granting consent.")
    state = secrets.token_urlsafe(32)
    verifier = secrets.token_urlsafe(64)
    captured = {}
    received = threading.Event()

    class Callback(BaseHTTPRequestHandler):
        def do_GET(self):  # noqa: N802 - HTTP method name required by stdlib
            parsed = urllib.parse.urlparse(self.path)
            params = urllib.parse.parse_qs(parsed.query)
            captured["state"] = params.get("state", [""])[0]
            captured["code"] = params.get("code", [""])[0]
            captured["error"] = params.get("error", [""])[0]
            received.set()
            body = b"<html><body><h2>Kerna is connected.</h2><p>You can close this tab and return to Kerna.</p></body></html>"
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, _format, *_args):
            return

    server = ThreadingHTTPServer(("127.0.0.1", 0), Callback)
    redirect_uri = "http://127.0.0.1:%s/kerna-oauth" % server.server_port
    scope = WRITE_SCOPE if allow_write else READ_SCOPE
    query = urllib.parse.urlencode(
        {
            "client_id": client_id,
            "redirect_uri": redirect_uri,
            "response_type": "code",
            "scope": scope,
            "access_type": "offline",
            "prompt": "consent",
            "state": state,
            "code_challenge": _code_challenge(verifier),
            "code_challenge_method": "S256",
        }
    )
    print("Opening Google consent in your browser. Requested scope: %s" % ("calendar events read/write" if allow_write else "calendar events read-only"))
    webbrowser.open(AUTHORIZE_URL + "?" + query, new=1)
    server.timeout = 180
    while not received.is_set():
        server.handle_request()
        if not received.is_set():
            raise RuntimeError("Timed out waiting for Google consent. Run connect.py again to retry.")
    server.server_close()
    if captured.get("state") != state:
        raise RuntimeError("OAuth state check failed; no credentials were saved.")
    if captured.get("error"):
        raise RuntimeError("Google consent was not completed (%s)." % captured["error"])
    if not captured.get("code"):
        raise RuntimeError("Google did not return an authorization code.")
    refresh_token = _token_exchange(client_id, redirect_uri, verifier, captured["code"])
    _save_windows_user_env("KERNA_GOOGLE_CALENDAR_CLIENT_ID", client_id) if save else None
    saved = _save_windows_user_env("KERNA_GOOGLE_CALENDAR_REFRESH_TOKEN", refresh_token) if save else False
    if saved:
        print("Google Calendar connected. Restart Kerna so it receives the new user-environment secrets.")
        return
    if print_refresh_token:
        print("Set these values in a secure shell, then start Kerna from that shell:")
        print("KERNA_GOOGLE_CALENDAR_CLIENT_ID=%s" % client_id)
        print("KERNA_GOOGLE_CALENDAR_REFRESH_TOKEN=%s" % refresh_token)
        return
    raise RuntimeError(
        "Consent succeeded but this platform has no built-in persistent environment writer. "
        "Re-run with --print-refresh-token only in a secure terminal to set the two environment variables yourself."
    )


def main():
    parser = argparse.ArgumentParser(description="Connect the Kerna Google Calendar MCP plugin with OAuth + PKCE.")
    parser.add_argument("--client-id", default=os.environ.get("KERNA_GOOGLE_CALENDAR_CLIENT_ID", ""), help="Google OAuth Desktop client ID")
    parser.add_argument("--allow-write", action="store_true", help="Request calendar.events write access instead of the default read-only scope")
    parser.add_argument("--save", action="store_true", help="On Windows, save values to the current user's environment")
    parser.add_argument("--print-refresh-token", action="store_true", help="Print credentials only for manual secure-shell setup (avoid unless necessary)")
    args = parser.parse_args()
    if not args.client_id.strip():
        parser.error("--client-id is required (create a Google OAuth Desktop client first)")
    try:
        connect(args.client_id.strip(), args.allow_write, args.save, args.print_refresh_token)
    except RuntimeError as exc:
        print("Connection not saved: %s" % exc, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
