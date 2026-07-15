# Kerna cohort launch checklist

Use this checklist to admit the first real-user cohort. A checked engineering
test is not a substitute for a checked live-user or live-connector outcome.
The release owner records the date, workspace, and safe task ID for every live
check below; never attach raw databases, OAuth tokens, or private calendar
content.

## Verified before inviting users

| Gate | Evidence | Status |
| --- | --- | --- |
| Fail-closed tool policy, scoped routines, manifests, trace redaction, and approval queue | Kernel tests and a separate approval-queue calendar workflow | Verified in the local test workspace |
| Useful local write remains approval-gated | Fresh productivity-pack run of `MOCK_NOTES_ADD`: approve queue entry, then inspect the completed receipt and sandboxed Markdown note | Verified in the local test workspace |
| Curated productivity pack | Fresh `kerna init --ci --provider mock` followed by `kerna pack install productivity` | Verified |
| Google Calendar pack is fail-closed | Pack unit test plus fresh `kerna pack install google-workspace` and `kerna mcp risk google-calendar` | Verified |
| Google connector contract | Manifest limits tools, secrets, network hosts, and requires approval for creation | Verified by manifest and plugin tests |
| OAuth setup safety | PKCE loopback helper defaults to `calendar.events.readonly`; it will not begin consent without an explicit storage choice | Verified by plugin test |
| Desktop control surface | Task receipts, local approval queue, routine scope, connector setup state, and latest tool-call result are represented | Desktop build and responsive visual smoke check verified |
| Controlled-cohort artifact integrity | v0.2.3 published CLI and native desktop assets with SHA-256 checksum files; all 20 released assets, five CLI/plugin-bundle sidecars, and six desktop-installer manifest entries were verified against their SHA-256 values | Verified on 2026-07-15 |

## Required live acceptance checks

Run these with a disposable Google account and disposable calendar before any
personal or team account is connected.

1. Install the connector, inspect its risk card, and run `connect.py` without
   `--allow-write`. Confirm Google’s consent screen requests only the
   read-only Calendar events scope.
2. Restart the desktop app. Run `google_calendar_status` and
   `google_list_events` against the disposable calendar. Confirm the task
   receipt is completed and the desktop connector row reports a successful
   last tool call. To avoid model ambiguity and model-key setup during this
   check, use `kerna run --approval-queue MOCK_GOOGLE_CALENDAR_STATUS` and
   `kerna run --approval-queue MOCK_GOOGLE_CALENDAR_LIST`; approve each queued
   read in the local queue.
3. Ask to create a disposable event after reconnecting with `--allow-write`.
   Confirm that the action enters the local queue, is absent from the calendar
   until approval, and is created once after approval with no invitations.
   `kerna run --approval-queue MOCK_GOOGLE_CALENDAR_CREATE` provides a fixed,
   future disposable event payload for this exact check. Verify the queued
   payload says `send_updates: none`, then delete the test event afterward.
4. Deny a second creation request. Confirm no event is created.
5. Revoke the Google grant. Run a read tool and confirm Kerna records a failed
   connector result that asks for reconnect, without recording a token or
   provider response body.
6. Disconnect the network during a read. Confirm the result is a bounded,
   understandable failure, not a retry loop or a successful-looking receipt.

Record only the task IDs, outcome, platform, and elapsed time in the
[cohort acceptance record](COHORT_ACCEPTANCE_RECORD.md). If any check fails,
stop expansion and file the receipt as a launch blocker.

## Controlled-cohort distribution checks

1. Create the cohort tag and confirm the release contains the platform-specific
   CLI, desktop installer, the curated `kerna-plugins.zip` bundle, and a
   corresponding SHA-256 checksum file for each downloaded artifact.
2. Verify the downloaded CLI checksum before installing it. Verify the desktop
   installer against the release's `SHA256SUMS` file before sharing it with a
   cohort member.
3. On Windows, confirm the installer starts the expected Kerna application and
   the CLI is installed first. Set `KERNA_HOME` to the initialized workspace
   before opening the desktop app.
4. Record the release tag and checksum value in the cohort invitation. Do not
   distribute an artifact from a local build folder.

Use the [cohort acceptance record](COHORT_ACCEPTANCE_RECORD.md) to retain the
verified tag, checksum filenames, and test outcomes without collecting private
calendar content or credentials.

Code signing and macOS notarization remain required before a broad public
release. They are not silently implied by a successful unsigned build; this
controlled cohort uses a known release URL and verified checksums instead.

## Cohort operating rules

- Start with 5–10 technically comfortable users on local notes, calendar, and
  read-only routines. Google Calendar remains opt-in and uses a test account
  for its first connection.
- Do not grant `auto_approve` to hosted write or messaging tools. Keep routine
  allowlists read-only and narrowly named.
- Every user must be able to find the task receipt, connector state, routine
  state, and approval queue in the desktop app before they automate work.
- Collect: time to first useful receipt, successful routine rate, approval
  clarity, connector failure/reconnect rate, and trace-redaction reports.
- Pause the cohort if a task trace contains a credential, an action is executed
  without its reviewed policy, or a connector result is materially misleading.

## Expansion criteria

The initial cohort may expand only when the live checks all pass, each user can
reach a useful local result in 15 minutes, and no high-severity policy or
privacy issue remains open. Adding Gmail, Microsoft 365, Slack, Notion, Drive,
shared policy, or unattended writes is a separate release—not an implied
extension of this cohort.
