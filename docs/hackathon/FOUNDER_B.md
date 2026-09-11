# 🔥 Kerna Gauntlet — Founder B Plan

> **Role:** Evidence and Demo Captain
> **Branch:** `hackathon/telemetry-demo`
> **Goal:** Turn every agent run into credible, queryable evidence and make the three-minute story judge-ready.

## Your mission

Build the Hotdata lifecycle and telemetry layer, measure the real system, create the judge-facing story, and hand Founder A tested commits without touching the pipeline lane.

## Exclusive ownership

```text
scripts/hotdata_telemetry.py
demo/cases/**
docs/**
submission/**
README-HACKATHON.md
```

Exception: Founder A owns `docs/hackathon/decision-log.md`. You own `docs/hackathon/founder-b-status.md`.

Do not edit pipelines, Kerna workspaces, runtime scripts, or contracts after the minute-20 freeze.

## Paste this as the first message in your Codex task

```text
You are the evidence, telemetry, and demo engineer for Kerna Gauntlet, a 4.5-hour hackathon build in the Kerna repository. Read the repository before editing. My branch is hackathon/telemetry-demo, based on origin/hackathon/kerna-gauntlet. I exclusively own scripts/hotdata_telemetry.py, demo/cases/**, docs/** except docs/hackathon/decision-log.md, submission/**, and README-HACKATHON.md. Founder A concurrently owns pipelines/**, demo/workspaces/**, scripts/prepare_demo.py, scripts/run_gauntlet.py, contracts/**, and final integration. Never edit or revert Founder A's files. Treat contracts/*.json as frozen after minute 20. Build real Hotdata create/load/query/full-text/delete behavior, persist cross-run telemetry, measure only actual runs, and turn the evidence into a concise three-minute demo. Run focused checks, commit working checkpoints, and report exact commands, results, changed files, and blockers. Never commit or paste secrets. The success criterion is a live Hotdata report proving the parallel agents used isolated task databases, Kerna blocked strong-policy unsafe writes, and telemetry led to one measurable improvement.
```

## Laptop and branch setup — 10 minutes maximum

Wait until Founder A confirms `hackathon/kerna-gauntlet` is pushed, then:

```powershell
git clone https://github.com/premxai/kerna.git
cd kerna
git fetch origin
git switch -c hackathon/telemetry-demo origin/hackathon/kerna-gauntlet
git push -u origin hackathon/telemetry-demo
```

- [ ] Add the clone as the primary folder of `Kerna Gauntlet — Founder B`.
- [ ] Create a local, uncommitted `.env`.
- [ ] Confirm `git status --short` is clean before editing.
- [ ] Confirm you can push the branch.

## Build sequence

### 0:00–0:20 — Establish the contract

- [ ] Review the result and telemetry schemas with Founder A.
- [ ] Check that the fields support real parallel/sequential and weak/strong comparisons.
- [ ] Freeze the schemas.
- [ ] Create `docs/hackathon/founder-b-status.md`.

**Gate:** You can push, the schemas are frozen, and ownership is unambiguous.

### 0:20–1:05 — Prove Hotdata deeply

- [ ] Authenticate with a temporary event token.
- [ ] Create a disposable database.
- [ ] Declare and load a small case table.
- [ ] Run a SQL query that affects the demo.
- [ ] Run a full-text query that affects the demo.
- [ ] Delete the disposable database.
- [ ] Create the persistent telemetry database with a 12-hour expiry.
- [ ] Give Founder A only the non-secret database ID.

**Gate:** Create → load → SQL → full-text → delete works, and the persistent telemetry database is queryable.

### 1:05–2:05 — Build evidence tooling

- [ ] Implement `hotdata_telemetry.py init`.
- [ ] Implement `hotdata_telemetry.py append` using the frozen event schema.
- [ ] Implement `hotdata_telemetry.py report`.
- [ ] Add role-specific cases for utility, mutation, and containment.
- [ ] Add queries for parallel versus sequential elapsed time.
- [ ] Add queries for weak versus strong harmful starts.
- [ ] Add a duplicate `call_hash` query.
- [ ] Add a full-text finding search.
- [ ] Start `README-HACKATHON.md` with architecture and honest pre-existing-work disclosure.

**Gate:** A synthetic fixture can be appended and reported, with the fixture clearly labeled as test data.

### 2:05–3:10 — Measure the real system

- [ ] Hand Founder A a tested commit SHA.
- [ ] Watch the real weak, strong, parallel, and sequential runs.
- [ ] Store their actual start/end times and run IDs.
- [ ] Verify each specialist has a distinct task database ID.
- [ ] Verify each Kerna result includes a trace ID or a documented missing field.
- [ ] Replace all synthetic examples in judge-facing output with real numbers.
- [ ] Update the README with commands that reproduce the measurements.

### 3:10–3:35 — Prove learning, not logging

- [ ] Query repeated `call_hash` values.
- [ ] Identify one real redundant mutation path.
- [ ] Give Founder A the exact evidence and affected specialist.
- [ ] After Founder A adds deduplication, query the new run.
- [ ] Save before/after call counts and confirm the findings remain equivalent.

### 3:35–4:30 — Package the story

- [ ] Finalize the three-minute demo script.
- [ ] Finalize the Discord submission post.
- [ ] Preload one live Hotdata SQL query and one full-text search.
- [ ] Rehearse your 1:45–2:40 speaking block twice.
- [ ] Open a final PR into `hackathon/kerna-gauntlet`.
- [ ] Send Founder A the final commit SHA and handoff template.

## Judge-facing story

Your 55-second section should prove three things:

1. Each parallel specialist used a separate Hotdata task world.
2. The strong Kerna policy reduced harmful starts to zero while useful work still passed.
3. Cross-run telemetry revealed repeated calls, and the team measurably removed them.

Show Hotdata driving case selection, isolation, analysis, and the improvement decision.

## Definition of done

- [ ] Hotdata create, load, SQL, full-text, and delete all run for real.
- [ ] Persistent telemetry covers at least three real sessions.
- [ ] Live reports compare execution mode and policy mode.
- [ ] Every number in the README comes from captured output.
- [ ] The full-text search returns a meaningful finding during rehearsal.
- [ ] The telemetry evidence causes one documented code change.
- [ ] The demo script and submission disclose Kerna as pre-existing work.
- [ ] Founder A integrates your final tested commit without resolving ownership conflicts.

## Commit rhythm

```text
feat(telemetry): add Hotdata lifecycle commands
feat(telemetry): report policy and execution comparisons
test(gauntlet): add specialist case datasets
docs(gauntlet): add architecture and prior-work disclosure
docs(gauntlet): add timed demo and submission copy
```

Update `docs/hackathon/founder-b-status.md` after each pushed checkpoint with the commit SHA, exact verification command, result, and what Founder A needs next.
