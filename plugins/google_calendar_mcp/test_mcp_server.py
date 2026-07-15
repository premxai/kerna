import importlib.util
import pathlib
import unittest
from unittest.mock import patch

MODULE_PATH = pathlib.Path(__file__).with_name("mcp_server.py")
SPEC = importlib.util.spec_from_file_location("google_calendar_mcp", MODULE_PATH)
SERVER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SERVER)

CONNECT_PATH = pathlib.Path(__file__).with_name("connect.py")
CONNECT_SPEC = importlib.util.spec_from_file_location("google_calendar_connect", CONNECT_PATH)
CONNECT = importlib.util.module_from_spec(CONNECT_SPEC)
CONNECT_SPEC.loader.exec_module(CONNECT)


class GoogleCalendarPluginTests(unittest.TestCase):
    def test_manifest_tools_and_create_contract_are_exposed(self):
        response = SERVER.handle({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})
        tools = {tool["name"]: tool for tool in response["result"]["tools"]}
        self.assertEqual(set(tools), {"google_calendar_status", "google_list_events", "google_create_event"})
        self.assertEqual(tools["google_create_event"]["inputSchema"]["properties"]["send_updates"]["default"], "none")

    def test_list_events_is_bounded_and_formats_provider_response(self):
        seen = {}

        def fake_request(url, method="GET", data=None, headers=None):
            seen.update({"url": url, "method": method, "headers": headers})
            return {"items": [{"summary": "Design review", "location": "Room 4", "start": {"dateTime": "2026-07-15T09:00:00-04:00"}}]}

        with patch.object(SERVER, "_access_token", return_value="access-token"), patch.object(SERVER, "_json_request", side_effect=fake_request):
            result = SERVER.google_list_events(max_results=500)
        self.assertIn("Design review", result)
        self.assertIn("maxResults=100", seen["url"])
        self.assertEqual(seen["headers"]["Authorization"], "Bearer access-token")

    def test_create_requires_offset_and_never_sends_notifications_by_default(self):
        with self.assertRaisesRegex(ValueError, "timezone offset"):
            SERVER.google_create_event("Review", "2026-07-15T09:00:00", "2026-07-15T09:30:00-04:00")
        with self.assertRaisesRegex(ValueError, "after start"):
            SERVER.google_create_event("Review", "2026-07-15T10:00:00-04:00", "2026-07-15T09:30:00-04:00")
        with patch.object(SERVER, "_access_token", return_value="access-token"), patch.object(SERVER, "_json_request", return_value={"summary": "Review", "start": {"dateTime": "2026-07-15T09:00:00-04:00"}}) as request:
            SERVER.google_create_event("Review", "2026-07-15T09:00:00-04:00", "2026-07-15T09:30:00-04:00")
        self.assertIn("sendUpdates=none", request.call_args.args[0])
        self.assertEqual(request.call_args.kwargs["method"], "POST")

    def test_access_token_errors_do_not_echo_provider_body(self):
        from urllib.error import HTTPError
        import io

        with patch.object(SERVER.urllib.request, "urlopen", side_effect=HTTPError(SERVER.TOKEN_URL, 400, "bad", {}, io.BytesIO(b'{"error_description":"token-secret"}'))), patch.dict("os.environ", {"KERNA_GOOGLE_CALENDAR_CLIENT_ID": "id", "KERNA_GOOGLE_CALENDAR_REFRESH_TOKEN": "refresh"}, clear=True):
            with self.assertRaisesRegex(ValueError, "expired or was revoked") as error:
                SERVER._access_token()
        self.assertNotIn("token-secret", str(error.exception))

    def test_setup_refuses_to_start_consent_without_a_safe_storage_choice(self):
        with self.assertRaisesRegex(RuntimeError, "Choose --save"):
            CONNECT.connect("client-id", allow_write=False, save=False, print_refresh_token=False)


if __name__ == "__main__":
    unittest.main()
