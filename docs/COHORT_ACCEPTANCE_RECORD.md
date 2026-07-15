# Kerna cohort acceptance record

Copy this file to a private, access-controlled launch record before inviting
users. It deliberately stores only safe operational evidence. Never place OAuth
client secrets, refresh tokens, calendar titles, event details, raw database
files, task payloads, or trace exports here.

## Release and environment

| Field | Value |
| --- | --- |
| Release tag | `v0.2.3` |
| Release URL | `https://github.com/premxai/kerna/releases/tag/v0.2.3` |
| Verification date (UTC) | |
| Release owner | |
| CLI platform and filename | |
| CLI SHA-256 sidecar filename | |
| Desktop platform and installer filename | |
| Desktop SHA256SUMS filename | |
| CLI checksum verified | Pass / Fail |
| Desktop checksum verified | Pass / Fail |
| Disposable Google account used | Yes / No (do not record its address) |
| Disposable calendar used | Yes / No (do not record its name) |

Record the checksum filenames and the Pass/Fail result, not the downloaded
artifact itself. Any failed checksum is an immediate distribution blocker.

## Live Google Calendar acceptance

Run each check in [the cohort launch checklist](COHORT_LAUNCH_CHECKLIST.md)
with a disposable account and calendar. Task IDs are safe only after reviewing
their trace for secrets before sharing them.

| Check | Date (UTC) | Platform | Safe task ID | Outcome | Elapsed time | Notes (no private content) |
| --- | --- | --- | --- | --- | --- | --- |
| Read-only consent requests only Calendar events read scope | | | N/A | Pass / Fail | | |
| Status read completes and desktop shows successful connector result | | | | Pass / Fail | | |
| Event list completes and desktop shows successful connector result | | | | Pass / Fail | | |
| Write request queues; no event exists before approval | | | | Pass / Fail | | |
| Approved write creates one event with `send_updates: none` | | | | Pass / Fail | | |
| Denied second write creates no event | | | | Pass / Fail | | |
| Revoked grant yields bounded reconnect guidance without secret leakage | | | | Pass / Fail | | |
| Offline read yields a bounded, understandable failure | | | | Pass / Fail | | |

## Cohort usability evidence

Each participant must complete the private-alpha test script before expansion.
Use an anonymous participant label; retain contact details elsewhere with their
consent.

| Participant label | OS | Date | First useful receipt within 15 min | Routine scope understood | Approval queue understood | Trace/secret issue | Blocker or follow-up |
| --- | --- | --- | --- | --- | --- | --- |
| | | | Yes / No | Yes / No | Yes / No | None / Describe safely | |
| | | | Yes / No | Yes / No | Yes / No | None / Describe safely | |
| | | | Yes / No | Yes / No | Yes / No | None / Describe safely | |
| | | | Yes / No | Yes / No | Yes / No | None / Describe safely | |
| | | | Yes / No | Yes / No | Yes / No | None / Describe safely | |

## Launch decision

| Gate | Result | Owner | Date (UTC) | Evidence reference |
| --- | --- | --- | --- | --- |
| All live Google checks pass | Pass / Fail | | | |
| Five to ten technical users complete the alpha script | Pass / Fail | | | |
| Each participant reaches a useful local result within 15 minutes | Pass / Fail | | | |
| No high-severity policy or privacy issue remains open | Pass / Fail | | | |
| Initial-cohort expansion approved | Yes / No | | | |

Do not mark the final row `Yes` if any preceding gate is incomplete or failed.
Open a launch blocker for any credential leak, action outside reviewed policy,
misleading connector result, failed checksum, or incomplete live check.
