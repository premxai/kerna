# Kerna Private Alpha Guide

Kerna's first cohort is for people who want useful AI-assisted work without
giving an agent a blank check. This alpha validates a local-first productivity
loop: connect small MCP tools, run a bounded task or routine, and inspect the
receipt afterward.

## What is in scope

- Local Markdown notes, local iCalendar files, weather, web reading, and
  optional search through the curated `productivity` pack.
- Optional Google Calendar through the curated `google-workspace` pack. The
  consent helper requests an OAuth read-only scope by default; event creation
  needs a separate write grant and per-action approval.
- Manual tasks that use the normal fail-closed policy, terminal approval, or
  the local desktop approval queue.
- Read-only background routines: Morning Brief, Meeting Prep, Research Brief,
  Daily Digest, Morning News, and Weekly Review.
- A local desktop surface for starting tasks and reviewing task receipts,
  configured routines, connectors, and approved memories.
- Trace, policy, budget, manifest, and redaction behavior.

## What is not in scope yet

- OAuth-connected Gmail, Microsoft 365, Slack, Notion, Drive, and other hosted
  work accounts. Google Calendar is the one reviewed hosted connector in this
  cohort and must be trialled with a non-production account first.
- Unattended write/send/delete/publish actions.
- Hosted OAuth connectors and broad approval batching. The desktop alpha has a
  local, one-action-at-a-time approval queue; every request expires after five
  minutes if you do not decide.
- An enterprise shared-policy or remote-control plane.

Do not use a personal production mailbox password. The reference email plugin
is an IMAP/SMTP example and is not a launch connector.

## First useful run

From an empty workspace:

```bash
kerna init                         # choose your provider or Demo mode
kerna pack install productivity    # local notes/calendar/weather + optional web/search
kerna mcp list                     # verify the configured plugins
kerna routine add morning-brief    # creates a paused, scoped routine
kerna routine preview 0            # inspect tools and required policy
```

To connect the optional Google Calendar plugin, create a Google OAuth Desktop
client with the Calendar API enabled, then run:

```bash
kerna pack install google-workspace
kerna mcp risk google-calendar
python plugins/google_calendar_mcp/connect.py --client-id "YOUR_CLIENT_ID" --save
```

`connect.py` uses an authorization-code + PKCE flow and requests a read-only
calendar scope unless you deliberately add `--allow-write`. Restart Kerna after
Windows `--save` setup so the desktop app inherits the new user environment.

For the disposable-account acceptance steps, these deterministic goals use the
real Google connector through Kerna's scheduler and local approval queue, but
do not require a paid model to choose the tool:

```bash
kerna run --approval-queue MOCK_GOOGLE_CALENDAR_STATUS
kerna run --approval-queue MOCK_GOOGLE_CALENDAR_LIST
# Reconnect with --allow-write before this one.
kerna run --approval-queue MOCK_GOOGLE_CALENDAR_CREATE
```

Approve each queued read only after checking its receipt. The create goal uses
a fixed 2030 test event and `send_updates: none`; approve it only in the
disposable calendar and delete it after recording the result.

## Use the desktop control surface

The desktop app is a local view of the same workspace, database, policy, and
approval queue used by the CLI. It does not create a second account or copy
your data. Install the Kerna CLI first, then install the desktop package for
your platform from the same GitHub release. The app automatically looks in the
CLI installer's default user location (`~/.local/bin`; `%USERPROFILE%\\.local\\bin`
on Windows), then on `PATH`.

Set `KERNA_HOME` to the initialized workspace before launching the installed
app. Set `KERNA_BIN` only if the CLI is installed elsewhere. For source-based
cohort testing, use:

```powershell
cd ui
npm ci
$env:KERNA_HOME = "C:\path\to\your\kerna-workspace"
# Set this only when the CLI is not in its default location or on PATH.
$env:KERNA_BIN = "C:\path\to\kerna.exe"
npm run tauri dev
```

`KERNA_HOME` must contain the `kerna.toml` created by `kerna init`. The app
will show an error instead of guessing if it cannot find that workspace. Start
a task from the app to use the local approval queue; review the resulting
receipt, routines, and connector health in the same window.

To run a routine unattended, grant only its displayed read tools as
`auto_approve` in `kerna.toml`, then enable it explicitly:

```bash
kerna routine enable 0
kerna routine run 0                # verify the scoped routine immediately
kerna daemon
```

Kerna refuses to enable a routine with no reviewed tool allowlist. It also
refuses approval-required calls in the daemon, rather than waiting forever or
performing them without a person present.

## Cohort test script

Please complete these tasks and record whether the output was useful:

1. Run a manual task using a local tool, such as reading or searching a note;
   then capture a short note and verify it waits for your approval before it is
   written.
2. Add a Morning Brief, inspect its allowlist, and confirm it remains paused
   until you deliberately authorize its exact read tools.
3. Enable the briefing after reviewing policy, then inspect the resulting task
   receipt with `kerna trace last` and `kerna inspect last`.
4. Attempt a tool outside the routine allowlist or an approval-required action
   in a background context; it must be denied, not silently run.
5. Review the desktop surface: can you find the task, its policy events, the
   routine state, and configured connectors without reading the database?
6. If using the Google Calendar connector, use a disposable test calendar:
   confirm a read appears in its receipt, then verify an event-creation request
   stays queued until you approve it and defaults to no invitations.

## Feedback to include

For a bug report, include:

- Kerna version and OS;
- the safe-to-share goal text and exact command;
- the task id plus `kerna trace <task-id>` output (verify it contains no
  credentials before sharing);
- whether the issue was reliability, usefulness, policy clarity, or setup
  friction.

Never send API keys, OAuth tokens, email contents, or raw database files.

## Alpha exit criteria

Before expanding beyond this cohort, the team should have evidence that users
can reach one useful local receipt in 15 minutes, routines remain within their
reviewed read-only scope, task traces have no secret leakage, and the Google
Calendar flow has been tested end-to-end with a disposable account. OAuth mail
and collaboration connectors, richer approval risk summaries, and a shared
policy plane remain required release gates for a broader daily-productivity
launch.

Use the [cohort launch checklist](COHORT_LAUNCH_CHECKLIST.md) to record the
required live-account and user evidence before inviting or expanding users.
