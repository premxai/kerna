# Kerna Gauntlet agent instructions

These instructions apply to the entire repository on the hackathon branches.

## Read first

1. Read `CLAUDE.md` for Kerna's architecture, commands, and fail-closed invariant.
2. Read `docs/hackathon/START_HERE.md` for the current objective and operating model.
3. Read `docs/hackathon/ROCKETRIDE_ROLE.md` before changing any pipeline.
4. Read only the founder brief for the active branch:
   - `hackathon/core-pipeline` → `docs/hackathon/FOUNDER_A.md`
   - `hackathon/telemetry-demo` → `docs/hackathon/FOUNDER_B.md`
5. Treat `contracts/*.schema.json` as the integration boundary.

## Objective

Build Kerna Gauntlet: a RocketRide parallel wave whose utility, mutation, and containment specialists use isolated Hotdata task databases and route filesystem tool calls through Kerna. Deploy the final `.pipe` to RocketRide Cloud and return an evidence-backed permission certificate comparing weak and strong policy behavior.

## Parallel ownership

Founder A owns:

- `pipelines/**`
- `demo/workspaces/**`
- `scripts/prepare_demo.py`
- `scripts/run_gauntlet.py`
- `contracts/**`
- `docs/hackathon/decision-log.md`
- final integration into `hackathon/kerna-gauntlet`

Founder B owns:

- `scripts/hotdata_telemetry.py`
- `demo/cases/**`
- `docs/**`, except `docs/hackathon/decision-log.md`
- `submission/**`
- `README-HACKATHON.md`

Never edit, revert, rename, or broadly format the other founder's owned files. Ask for a contract change through the handoff protocol; Founder A makes the agreed contract edit.

## Engineering rules

- Preserve Kerna's fail-closed permission invariant.
- Keep domain logic outside the kernel unless the existing architecture requires it there.
- Use RocketRide, Hotdata, and Kerna materially in the critical path.
- Prefer a small deterministic demo path over optional features.
- Run focused checks for changed code and record the exact command and result.
- Commit working checkpoints every 25 minutes and push them to the assigned branch.
- Never force-push or rebase a shared branch.
- Never commit secrets, `.env`, tokens, database credentials, or copied browser sessions.
- Do not commit generated `.claude/` or `.github/copilot-instructions.md` files without explicit review.
- If `.rocketride/docs/` exists locally, read its README, pipeline rules, component reference, API guide, and common mistakes before writing RocketRide code. The directory is generated locally and ignored by Git.

## Completion report

Every implementation checkpoint must report:

```text
STATUS: READY TO INTEGRATE | BLOCKED | IN PROGRESS
OWNER: Founder A | Founder B
BRANCH: hackathon/...
COMMIT: <full SHA or NONE>
FILES CHANGED: <paths>
WHAT WORKS: <one sentence>
VERIFIED WITH: <exact command and result>
ENV NEEDED: <variable names only>
KNOWN LIMITATION: <one sentence>
```

Do not claim sponsor integration, parallel execution, enforcement, or performance improvement without evidence from an actual run.
