# 🧠 Kerna Gauntlet — Founder A Plan

> **Role:** Integration Captain
> **Branch:** `hackathon/core-pipeline`
> **Goal:** Make the end-to-end RocketRide → Hotdata → Kerna certification run work twice from a clean command.

## Your mission

Build the real multi-agent execution path, enforce unsafe tool calls through Kerna, integrate Founder B’s telemetry, and protect the last known-good demo.

## Exclusive ownership

```text
pipelines/**
demo/workspaces/**
scripts/prepare_demo.py
scripts/run_gauntlet.py
contracts/**
docs/hackathon/decision-log.md
```

Do not edit Founder B’s telemetry, test cases, README, or submission files except while resolving an integration conflict together.

## Paste this as the first message in your Codex task

```text
You are the implementation and integration engineer for Kerna Gauntlet, a 4.5-hour hackathon build in the Kerna repository. Read the repository before editing. My branch is hackathon/core-pipeline. I exclusively own pipelines/**, demo/workspaces/**, scripts/prepare_demo.py, scripts/run_gauntlet.py, contracts/**, and docs/hackathon/decision-log.md. Founder B concurrently owns scripts/hotdata_telemetry.py, demo/cases/**, docs/** except decision-log.md, submission/**, and README-HACKATHON.md. Never edit or revert Founder B's files. Treat contracts/*.json as frozen after minute 20 unless I explicitly approve a change. Keep the live demo path small, use real sponsor products materially, run focused checks after changes, commit working checkpoints, and report exact commands, results, changed files, and blockers. Do not commit secrets or unreviewed RocketRide-generated instruction files. The success criterion is a real RocketRide parallel wave whose specialists use separate Hotdata databases and whose filesystem actions pass through Kerna, followed by an evidence-backed certificate.
```

## Start-state check — 10 minutes maximum

- [ ] Open the project at the Kerna repo root.
- [ ] Run `git status -sb` and confirm `hackathon/kerna-gauntlet` contains the two hackathon commits.
- [ ] Review the `.gitignore` change and untracked `.claude/` and `.github/copilot-instructions.md` files.
- [ ] Push `hackathon/kerna-gauntlet`.
- [ ] Create and push `hackathon/core-pipeline`.
- [ ] Tell Founder B the base branch is ready.
- [ ] Create the two frozen JSON schemas.

## Build sequence

### 0:00–0:20 — Establish the contract

- [ ] Freeze `agent-result.schema.json` with Founder B.
- [ ] Freeze `telemetry-event.schema.json` with Founder B.
- [ ] Create `docs/hackathon/decision-log.md`.
- [ ] Record branch names, owners, and fallback decisions.

**Gate:** Founder B can clone the pushed base and both schemas are agreed.

### 0:20–1:05 — Prove the sponsor seams

- [ ] Start RocketRide and select a working model.
- [ ] If the managed model is blocked for ten minutes, use Ollama `qwen3:8b`.
- [ ] Recover Docker once; do not spend the sprint repeatedly debugging it.
- [ ] Run Kerna’s filesystem acceptance test twice.
- [ ] Make one minimal RocketRide pipeline discover and call one Kerna MCP tool.
- [ ] Receive the non-secret Hotdata telemetry database ID from Founder B.

**Gate:** One real MCP call is visible in RocketRide and a Kerna receipt.

### 1:05–2:05 — Build the Gauntlet

- [ ] Prepare weak and strong workspaces for utility, mutation, and containment.
- [ ] Build the three specialist agents.
- [ ] Give each specialist a distinct Hotdata task database.
- [ ] Route filesystem actions through a Kerna MCP Client.
- [ ] Build `gauntlet-parallel.pipe` with one concurrent wave.
- [ ] Return strict JSON matching the frozen result schema.
- [ ] Add deterministic merge logic before any optional generated summary.

**Gate:** A real parallel run produces three specialist results, even if the presentation is still rough.

### 2:05–3:10 — Integrate and measure

- [ ] Review Founder B’s PR or commit SHA.
- [ ] Integrate only the tested telemetry commit.
- [ ] Run parallel weak and parallel strong.
- [ ] Build a sequential pipeline using identical cases, model, and prompts.
- [ ] Verify useful read succeeds in both modes.
- [ ] Verify at least one weak unsafe write starts.
- [ ] Verify every strong unsafe write is stopped before execution.
- [ ] Verify task database IDs are distinct and lifecycle cleanup runs.

### 3:10–3:35 — Make one evidence-driven improvement

- [ ] Use Founder B’s duplicate-call query to find a real repeated mutation path.
- [ ] Add canonical-call deduplication.
- [ ] Run strong parallel again.
- [ ] Keep the change only if the same findings require fewer calls.

### 3:35–4:30 — Freeze and perform

- [ ] Merge final Founder B docs and telemetry.
- [ ] Freeze `hackathon/kerna-gauntlet`.
- [ ] Run the complete demo twice.
- [ ] Keep RocketRide canvas, Kerna receipt, and one Hotdata query preloaded.
- [ ] Push the final branch and verify it from an incognito browser.
- [ ] Rehearse your 0:00–1:45 and 2:40–3:00 speaking blocks.

## Automatic scope cuts

| Trigger | Cut |
|---|---|
| Managed model unavailable after 10 minutes | Use Ollama |
| Docker recovery exceeds 10 minutes | Use the verified fallback and disclose it |
| No real parallel run by 2:05 | Remove policy synthesis |
| Integration unstable at 3:35 | Remove optional visuals and freeze the known-good CLI path |

## Definition of done

- [ ] One command launches the demo.
- [ ] Three specialists visibly execute in one RocketRide wave.
- [ ] Every specialist materially loads and queries a distinct Hotdata database.
- [ ] Every filesystem action goes through Kerna.
- [ ] Weak and strong runs visibly differ at the enforcement point.
- [ ] A real Kerna trace ID appears in the result.
- [ ] Founder B’s live report reads the same run IDs.
- [ ] The full demo works twice without editing code between runs.

## Commit rhythm

```text
feat(gauntlet): add frozen result contracts
feat(gauntlet): add isolated weak and strong workspaces
feat(gauntlet): run three specialists in RocketRide wave
feat(gauntlet): route filesystem tests through Kerna
chore(gauntlet): integrate Hotdata evidence reporting
```

Update `docs/hackathon/founder-a-status.md` after each pushed checkpoint with the commit SHA, exact verification command, result, and next blocker.
