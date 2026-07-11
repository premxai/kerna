# Kerna plugins

Kerna owns no domain logic — every capability is an MCP plugin spawned as an isolated child process. This directory ships reference plugins so you have useful tools on day one. They speak standard MCP over stdio and each carries a `manifest.toml` (risk card).

Nothing loads automatically: Kerna is fail-closed, so you add the plugins you want and grant each tool explicitly.

## Starter pack (zero dependencies — Python standard library only)

| Plugin | Tools | Notes |
|--------|-------|-------|
| **files** | `read_file`, `write_file`, `list_dir`, `search_text` | Confined to the workspace; can't escape the sandbox boundary. |
| **web** | `fetch_url`, `read_page_text` | http/https only, size- and time-bounded. `read_page_text` strips HTML. |
| **git** | `git_status`, `git_branch`, `git_log`, `git_diff` | Read-only git — never mutates the repo or touches the network. |

Add all three at once:

```bash
# from your project directory — form is: kerna mcp add <name> <command> [args...]
kerna mcp add files python "<KERNA_DIR>/plugins/files_mcp/mcp_server.py"
kerna mcp add web   python "<KERNA_DIR>/plugins/web_mcp/mcp_server.py"
kerna mcp add git   python "<KERNA_DIR>/plugins/git_mcp/mcp_server.py"
```

Or run the one-shot helper (from the Kerna repo root):

```bash
./scripts/add_starter_plugins.sh      # macOS/Linux
scripts\add_starter_plugins.ps1       # Windows
```

Then inspect and grant:

```bash
kerna mcp list
kerna mcp risk files          # read the risk card before granting anything
```

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
