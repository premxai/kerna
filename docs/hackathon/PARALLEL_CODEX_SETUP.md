# 🛡️ Kerna Gauntlet — Parallel Codex Setup

> **Decision:** Create a **Kerna Gauntlet local project on each laptop**. Give each founder one long-running Codex task, one clone, one branch, and one file-ownership lane.

## What “same project” means

The two projects may have the same name and point to the same GitHub repository, but each laptop has its **own local clone and filesystem**. Codex tasks do not automatically share conversation memory or uncommitted changes. GitHub is the synchronization layer.

```text
Founder A laptop                          Founder B laptop
Kerna Gauntlet local project              Kerna Gauntlet local project
        ↓                                         ↓
hackathon/core-pipeline branch             hackathon/telemetry-demo branch
        └──────────── commits / PRs ─────────────┘
                          ↓
              hackathon/kerna-gauntlet
                 integration branch
```

## Account requirement

If both founders use Codex at the same time, each should sign in with a separate authorized OpenAI account or workspace member identity. Do not build the workflow around sharing one personal login.

If only one Codex identity is available today:

- Founder A runs the Codex task and owns integration.
- Founder B follows the Founder B page in VS Code, GitHub, RocketRide, Hotdata, and local tools.
- Founder B sends commit SHAs and exact errors to Founder A.

## Create the two local projects

### Founder A laptop

1. Add a **local project** named `Kerna Gauntlet — Founder A`.
2. Select this repository root as the primary folder:

```text
C:\Users\kanap\Documents\Codex\2026-09-10\so-x20\work\kerna-gauntlet
```

3. Use one **Local** Codex task for implementation so changes remain visible in the same checkout.
4. Do not choose the broad `so-x20` folder as the project root; Git operations and repository instructions must resolve from the Kerna repo.

### Founder B laptop

1. Clone `https://github.com/premxai/kerna.git` into a clean folder.
2. Add a **local project** named `Kerna Gauntlet — Founder B`.
3. Select that clone’s repository root as the primary folder.
4. Use one **Local** Codex task for implementation.

## Git setup — do this before feature work

Founder A’s current hackathon branch contains two local commits and has not been pushed. It also has RocketRide-generated uncommitted files. Founder A should review and separate those files before the first push.

Founder A:

```powershell
git status --short
git push -u origin hackathon/kerna-gauntlet
git switch -c hackathon/core-pipeline
git push -u origin hackathon/core-pipeline
```

Founder B, after Founder A confirms the base branch is pushed:

```powershell
git clone https://github.com/premxai/kerna.git
cd kerna
git fetch origin
git switch -c hackathon/telemetry-demo origin/hackathon/kerna-gauntlet
git push -u origin hackathon/telemetry-demo
```

## Pages to import into Notion

- `Kerna Gauntlet - Founder A Plan.md`
- `Kerna Gauntlet - Founder B Plan.md`
- `Kerna Gauntlet - Two-Founder Notion Playbook.md`

Import all three Markdown files into one Notion page called **Kerna Gauntlet HQ**. Put the two founder pages side by side at the top.

## Fixed ownership map

| Area | Founder A | Founder B |
|---|---:|---:|
| Kerna gateway and policies | ✅ | — |
| RocketRide pipelines and agents | ✅ | — |
| Runtime and integration scripts | ✅ | — |
| Hotdata telemetry implementation | — | ✅ |
| Test cases and measurements | — | ✅ |
| README, demo script, submission | — | ✅ |
| Contract changes | Commits after both agree | Reviews |
| Final integration branch | ✅ | — |

## Files that prevent chat-memory drift

Create these in the repository during the first 20 minutes:

```text
contracts/agent-result.schema.json
contracts/telemetry-event.schema.json
docs/hackathon/founder-a-status.md
docs/hackathon/founder-b-status.md
docs/hackathon/decision-log.md
```

Each founder edits only their own status page. Founder A owns the decision log. This avoids both Codex tasks changing the same coordination file.

## Coordination clock

| When | Founder A | Founder B |
|---|---|---|
| Every 25 minutes | Commit and push working state | Commit and push working state |
| Every 50 minutes | Merge or cherry-pick a tested handoff | Open/update PR and post SHA |
| At 2:05 elapsed | Produce first real parallel result | Produce first real telemetry report |
| At 3:35 elapsed | Freeze code | Freeze copy and demo assets |
| Final 55 minutes | Integrate, test, rehearse | Query, narrate, rehearse |

## Handoff protocol

Paste this into Notion and the PR description:

```text
STATUS: READY TO INTEGRATE
OWNER: Founder A / Founder B
BRANCH: hackathon/...
COMMIT: <full SHA>
FILES CHANGED: <paths>
WHAT WORKS: <one sentence>
VERIFIED WITH: <exact command and result>
ENV NEEDED: <variable names only>
KNOWN LIMITATION: <one sentence>
```

## Non-negotiable rules

1. One owner per file group.
2. Never force-push.
3. Never rebase a branch after another founder has based work on it.
4. Never paste secrets into Codex, Notion, Git, screenshots, or Discord.
5. Commit only files you intentionally reviewed.
6. Do not commit RocketRide-generated `.claude/` or `.github/copilot-instructions.md` files until their contents are reviewed.
7. Do not let both Codex tasks refactor shared code during the sprint.
8. Founder A alone merges into `hackathon/kerna-gauntlet`.

## Final operating model

The founders work in parallel. The Codex tasks advise independently. The schemas keep their outputs compatible. Git records the truth. Founder A integrates only tested commits.
