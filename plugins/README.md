# Kerna plugins

Kerna owns no domain logic — every capability is an MCP plugin spawned as an isolated child process. This directory ships reference plugins so you have useful tools on day one. They speak standard MCP over stdio and each carries a `manifest.toml` (risk card).

Nothing loads automatically: Kerna is fail-closed, so you add the plugins you want and grant each tool explicitly.

## Tool packs (fastest way to get useful tools)

A pack installs a curated set of plugins in one command, declares any secrets,
and applies fail-closed permissions (read tools become
`require_confirmation`; nothing is auto-approved). The productivity pack is
local-first: notes and calendar work without cloud credentials, while search
is optional.

```bash
kerna pack list                    # productivity, dev
kerna pack install productivity    # search + notes + web
kerna secrets add search           # set the search API key it needs
kerna mcp risk search              # read the risk card
```

| Pack | Plugins |
|------|---------|
| **productivity** | search, notes, web, calendar, weather |
| **google-workspace** | Google Calendar via OAuth; read-only consent by default |
| **dev** | files, git, http |

## Zero-dependency plugins (Python standard library only)

| Plugin | Tools | Notes |
|--------|-------|-------|
| **files** | `read_file`, `write_file`, `list_dir`, `search_text` | Confined to the workspace; can't escape the sandbox boundary. |
| **web** | `fetch_url`, `read_page_text` | http/https only, size- and time-bounded. `read_page_text` strips HTML. |
| **git** | `git_status`, `git_branch`, `git_log`, `git_diff` | Read-only git — never mutates the repo or touches the network. |
| **search** | `web_search` | Web search via Tavily. Needs `TAVILY_API_KEY` (free at tavily.com). |
| **notes** | `add_note`, `list_notes`, `read_note`, `search_notes` | Markdown notes in a workspace `notes/` folder — nothing leaves your machine. |
| **http** | `http_get`, `http_post_json` | Generic REST/JSON caller. Optional `KERNA_HTTP_ALLOWLIST` restricts hosts. |
| **email** | `send_email`, `list_recent_emails`, `read_email` | IMAP/SMTP. Needs `EMAIL_ADDRESS` + `EMAIL_PASSWORD` (app password). `send_email` requires your approval. |
| **calendar** | `list_events`, `add_event` | Local iCalendar `.ics` file (`KERNA_CALENDAR_FILE`, default `./calendar.ics`). Nothing leaves your machine. |
| **google-calendar** | `google_calendar_status`, `google_list_events`, `google_create_event` | OAuth Google Calendar. Read-only consent by default; creating an event needs a write grant and per-action approval. |
| **weather** | `get_weather` | Current + 3-day outlook via wttr.in. No API key. |
| **sqlite** | `list_tables`, `sql_query` | Read-only SQL against `KERNA_SQLITE_DB` (SELECT/WITH/PRAGMA/EXPLAIN only). |

Add one manually with `kerna mcp add <name> <command> [args...]`, e.g. `kerna mcp add files python "<KERNA_DIR>/plugins/files_mcp/mcp_server.py"`.

**Real folders (Documents, Desktop, etc.):** the `files` plugin above stays sandbox-only. To reach a real folder, use Kerna's built-in `fs.read`/`fs.write`/`fs.list`/`fs.delete` tools instead (always available, no plugin needed) with `root: "<name>"` after granting it: `kerna folders add documents ~/Documents`. See the [everyday guide](../docs/USING_KERNA.md#real-files-and-documents).

Then inspect and grant:

```bash
kerna mcp list
kerna mcp risk files          # read the risk card before granting anything
```

For an unattended briefing, add a paused routine, inspect its exact read-only
tool allowlist, then explicitly enable it only after setting those read tools
to `auto_approve`:

```bash
kerna routine add morning-brief
kerna routine preview 0
kerna routine enable 0
```

## Google Calendar (OAuth)

The curated Google connector is separate from the local-first productivity
pack. It is an OAuth Desktop-client flow with PKCE; no Google credential is
ever written to `kerna.toml` or a task trace.

1. In Google Cloud, enable the Calendar API and create an OAuth **Desktop app**
   client. Copy its client ID.
2. Install the connector and inspect its risk card:

   ```bash
   kerna pack install google-workspace
   kerna mcp risk google-calendar
   ```

3. Connect with the default read-only scope. On Windows, `--save` stores the
   refresh token in your user environment; restart Kerna after consent.

   ```bash
   python plugins/google_calendar_mcp/connect.py --client-id "YOUR_CLIENT_ID" --save
   ```

   Use `--allow-write` only if you need event creation. Kerna still requires a
   separate approval for every `google_create_event`, and calendar invitations
   default to **no notifications**. On non-Windows systems, the helper asks you
   to deliberately opt into printing an environment value for secure-shell
   setup; it never writes a token to project files.

## Wrapped official servers (need Node/npx)

The whole public MCP ecosystem works — connect any server and Kerna governs it:

```bash
kerna mcp add fetch  npx -y @modelcontextprotocol/server-fetch
kerna mcp add github npx -y @modelcontextprotocol/server-github   # then: kerna secrets add github
kerna mcp add slack  npx -y @modelcontextprotocol/server-slack
```

Set each server's token with `kerna secrets add <name>` (declare the env var names under `secrets = [...]` on its `[[mcp_servers]]` entry), then risk-check and grant.

Grant the tools you want in `kerna.toml` (fail-closed — everything else stays denied):

```toml
[[permissions]]
tool = "read_file"
action = "auto_approve"

[[permissions]]
tool = "write_file"
action = "require_confirmation"   # pauses to ask you

[[permissions]]
tool = "read_page_text"
action = "auto_approve"
```

## Reference plugins (need extra packages)

| Plugin | Tools | Requires |
|--------|-------|----------|
| **desktop** | `desktop_click`, `desktop_type`, `fs_list_dir` | `pip install pyautogui` (only to actually control the desktop) |
| **voice** | `voice_speak`, `voice_listen` | `pip install pyttsx3 SpeechRecognition` |
| **mock** | `run_mock_action` | none — a tiny example |

These load and list their tools without the extra packages (imports are lazy); a tool call errors clearly if the package is missing.

## Writing your own

Copy `files_mcp/mcp_server.py` as a template. A Kerna-compatible MCP server handles four stdio JSON-RPC methods: `initialize`, `notifications/initialized`, `tools/list`, and `tools/call`. Add a `manifest.toml` declaring your capabilities so `kerna mcp risk` can render a risk card. Any third-party MCP server works too — e.g. `kerna mcp add fetch npx -y @modelcontextprotocol/server-fetch`.
