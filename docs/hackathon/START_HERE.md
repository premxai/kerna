# Kerna Gauntlet — Start Here

This branch is the complete handoff package for the two-founder hackathon build.

## Product

**Kerna Gauntlet is CI for agent permissions:** three parallel specialists test whether an AI agent can complete useful work without unauthorized mutation or network escape, then produce a Kerna-backed permission certificate.

## Track and sponsor use

| Item | Role in the product |
|---|---|
| Track | Multi-Agent / Parallel Agents |
| RocketRide | Runs the three specialists concurrently and merges their results |
| Hotdata | Supplies isolated task databases and persistent cross-run telemetry |
| Kerna | Enforces tool permissions before execution and records receipts |

The sponsor products belong in the runtime path. A decorative API call is not sufficient.

## Clone and create the Codex project

Founder B runs:

```powershell
git clone --branch hackathon/kerna-gauntlet https://github.com/premxai/kerna.git kerna-gauntlet
cd kerna-gauntlet
git switch -c hackathon/telemetry-demo
git push -u origin hackathon/telemetry-demo
```

Then add the cloned `kerna-gauntlet` folder as a **local Codex project** and make it the primary folder. Start one Local task in that project. Codex automatically discovers the root `AGENTS.md`.

Founder A uses the existing clone and creates `hackathon/core-pipeline` from `hackathon/kerna-gauntlet`.

## Read in this order

Founder A:

1. `AGENTS.md`
2. `CLAUDE.md`
3. `docs/hackathon/FOUNDER_A.md`
4. `docs/hackathon/COMMAND_CENTER.md`
5. `docs/hackathon/BLUEPRINT.md`
6. `contracts/agent-result.schema.json`
7. `contracts/telemetry-event.schema.json`

Founder B:

1. `AGENTS.md`
2. `CLAUDE.md`
3. `docs/hackathon/FOUNDER_B.md`
4. `docs/hackathon/COMMAND_CENTER.md`
5. `docs/hackathon/BLUEPRINT.md`
6. `contracts/agent-result.schema.json`
7. `contracts/telemetry-event.schema.json`

The full audit is in `docs/hackathon/READINESS_AUDIT.md`. Read it when diagnosing a blocker or challenging an assumption.

## Local environment

Create an untracked `.env` on each laptop from `.env.example`:

```dotenv
ROCKETRIDE_URI=
ROCKETRIDE_APIKEY=
HOTDATA_API_KEY=
HOTDATA_WORKSPACE_ID=
HOTDATA_TELEMETRY_DB_ID=
```

Use separate temporary service tokens when the sponsor dashboards support them. Share only non-secret IDs. Never put real values in Git, Notion, Codex messages, screenshots, or Discord.

Install and activate RocketRide on each laptop that needs its tooling. Its extension generates `.rocketride/docs/` locally. Docker Desktop has previously failed on Founder A's laptop with a stale `sailor-ingest.sock`; the decision table in `COMMAND_CENTER.md` limits recovery time.

## Branch and file ownership

| Branch | Owner | Files |
|---|---|---|
| `hackathon/core-pipeline` | Founder A | Pipeline, workspaces, run scripts, contracts |
| `hackathon/telemetry-demo` | Founder B | Hotdata telemetry, cases, documentation, submission |
| `hackathon/kerna-gauntlet` | Founder A | Tested integration and final submission |

Founder A merges. Founder B opens a PR or sends a tested commit SHA. Neither founder edits the other's owned files.

## First 20-minute gate

- [ ] Both founders can fetch and push their branch.
- [ ] Each founder has one clean local clone.
- [ ] Each founder has read their brief and pasted its starter prompt into their Codex task.
- [ ] Both JSON schemas are reviewed and frozen.
- [ ] Each laptop has an untracked `.env` with the required variable names.
- [ ] Each founder updates only their own status page.

No feature work begins until this gate passes.

## Handoff

Use the template in `AGENTS.md`. Founder A integrates only a green checkpoint: a pushed commit with an exact verification command and result.
