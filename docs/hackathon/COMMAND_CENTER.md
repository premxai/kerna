# 🛡️ Kerna Gauntlet — Two-Founder Hackathon Command Center

> **Mission:** Ship a premium, reliable demo that proves parallel agents can discover and repair unsafe AI-tool permissions before deployment.

| Property | Decision |
|---|---|
| 🏁 Track | Multi-Agent / Parallel Agents |
| ⏱️ Build window | 4.5 hours / 270 minutes |
| 👥 Team | Two founders, two laptops |
| 🧠 Codex | One task per founder with separate authorized identities; Founder A-only fallback |
| 🪐 Orchestration | RocketRide Cloud final deployment; local runtime for development/fallback |
| 🔥 Data layer | Hotdata task databases + persistent telemetry database |
| 🛡️ Enforcement | Kerna v0.2.9 MCP gateway |
| 🎛️ Demo surfaces | RocketRide canvas + Kerna dashboard + live Hotdata query |
| 🤖 Model | RocketRide-managed model; Ollama `qwen3:8b` fallback |
| 📦 Repository | `premxai/kerna`, branch `hackathon/kerna-gauntlet` |
| ⏰ Submission | 3:30 PM |

---

## 🚨 Team Account and Project Rule

> **Create a Kerna Gauntlet local project on each laptop, pointing to that laptop's own clone. Do not sign into the same personal ChatGPT/Codex account as two different people.**

OpenAI's account model assigns credentials and personal access tokens to an individual identity. Business and Enterprise collaboration uses separate member identities or dedicated service accounts rather than shared individual credentials. OpenAI's Business Terms also explicitly prohibit sharing individual login credentials between users. See [OpenAI access-token guidance](https://learn.chatgpt.com/docs/enterprise/access-tokens), [service-account guidance](https://learn.chatgpt.com/docs/enterprise/service-accounts), and [Business Terms](https://openai.com/policies/may-2025-business-terms/).

### Recommended arrangement for today

- **Founder A:** Uses a dedicated Codex task in the Founder A local project and owns integration-critical code.
- **Founder B:** Uses a dedicated Codex task in the Founder B local project when a separate authorized identity or seat is available.
- **Both:** Coordinate through Git commits and this Notion command center.
- **One-account fallback:** Founder A remains the Codex operator; Founder B uses VS Code, GitHub, RocketRide, Hotdata, browser documentation, and local tools.

The two tasks do not automatically share chat memory or uncommitted files. Frozen schemas, exclusive file ownership, and pushed commits keep them synchronized.

Import and assign these companion pages:

- `Kerna Gauntlet - Parallel Codex Setup.md`
- `Kerna Gauntlet - Founder A Plan.md`
- `Kerna Gauntlet - Founder B Plan.md`

---

## 🎯 Product in One Sentence

**Kerna Gauntlet is CI for agent permissions: three parallel specialists test whether an AI agent can finish its intended task without unauthorized mutation or network escape, then return an evidence-backed permission certificate.**

### The three specialists

| Specialist | Question it answers | Kerna tool | Success condition |
|---|---|---|---|
| ✅ Utility | Can the agent still perform the intended task? | `read_file` | Useful read succeeds under weak and strong policies |
| ✍️ Mutation | Can the agent modify state without review? | `write_file` | Weak starts the write; strong stops it before execution |
| 🌐 Containment | Can the tool escape to the network? | `network_probe` | Docker containment makes the network unavailable |

RocketRide launches all three in one wave. Each specialist receives a distinct Hotdata database. Kerna governs the real MCP calls. RocketRide merges the findings into one certification result.

---

## 👥 Ownership — No Shared Files During Parallel Work

### Founder A — Integration Captain

**Owns:** RocketRide, Kerna, pipelines, integration, final merge.

- [ ] Start and validate the RocketRide local runtime.
- [ ] Select RocketRide-managed model or activate Ollama fallback.
- [ ] Recover Docker and run the Kerna filesystem acceptance test.
- [ ] Prepare weak and strong Kerna workspaces.
- [ ] Build the three specialist agents and parent RocketRide Wave.
- [ ] Connect each specialist to a dedicated Kerna MCP Client.
- [ ] Build parallel and sequential `.pipe` files.
- [ ] Merge Founder B's telemetry branch.
- [ ] Run final end-to-end acceptance.

**Exclusive file ownership:**

```text
pipelines/**
demo/workspaces/**
scripts/prepare_demo.py
scripts/run_gauntlet.py
```

### Founder B — Evidence and Demo Captain

**Owns:** Hotdata telemetry, measurements, documentation, demo operations.

- [ ] Validate Hotdata credentials with create → load → query → delete.
- [ ] Create the persistent event-scoped telemetry database.
- [ ] Implement telemetry append and live report commands.
- [ ] Define the cross-run SQL and full-text queries.
- [ ] Record real sequential/parallel measurements.
- [ ] Maintain the judge-facing README and disclosure.
- [ ] Draft the three-minute demo script and Discord submission.
- [ ] Track blockers and integration-ready commit SHAs in Notion.

**Exclusive file ownership:**

```text
scripts/hotdata_telemetry.py
demo/cases/**
docs/**
submission/**
README-HACKATHON.md
```

### Shared contracts — Freeze after minute 20

Both founders review these once. Founder A commits them. Neither founder changes them independently afterward.

```text
contracts/agent-result.schema.json
contracts/telemetry-event.schema.json
.env.example
```

If a contract must change, both founders agree in Notion first, then Founder A makes the single change and gives Founder B the commit SHA.

---

## 🌿 Git Collaboration Model

### Branches

| Branch | Owner | Purpose |
|---|---|---|
| `hackathon/kerna-gauntlet` | Founder A | Integration and submission branch |
| `hackathon/core-pipeline` | Founder A | RocketRide and Kerna implementation |
| `hackathon/telemetry-demo` | Founder B | Hotdata telemetry, docs, and demo assets |

### Initial setup

Founder A:

```powershell
git push -u origin hackathon/kerna-gauntlet
git switch -c hackathon/core-pipeline
git push -u origin hackathon/core-pipeline
```

Founder B:

```powershell
git clone https://github.com/premxai/kerna.git
cd kerna
git fetch origin
git switch -c hackathon/telemetry-demo origin/hackathon/kerna-gauntlet
git push -u origin hackathon/telemetry-demo
```

### Rules that prevent breakage

1. **One owner per file group.** Do not make drive-by edits in the other founder's files.
2. **No force pushes.** Every shared commit remains recoverable.
3. **No rebasing after pushing.** Use merge commits or GitHub PRs.
4. **Commit every working checkpoint.** Prefer one purpose per commit.
5. **Push every 20–30 minutes.** A laptop failure must not erase the project.
6. **Founder A performs all integration merges.** Founder B never merges directly into the integration branch.
7. **No secrets in Git, Notion, ChatGPT, screenshots, or Discord.** Real values stay in each laptop's `.env`.
8. **A failed experiment goes on a new branch.** Do not destabilize the last known-good demo.

### Handoff template

Paste this into the Notion activity log whenever work is ready:

```text
STATUS: READY TO INTEGRATE
OWNER: Founder A / Founder B
BRANCH: hackathon/...
COMMIT: <full SHA>
FILES OWNED: <paths>
WHAT WORKS: <one sentence>
TEST RUN: <exact command and result>
ENV NEEDED: <variable names only>
KNOWN LIMITATION: <one sentence>
```

Founder A integrates using a PR or a targeted cherry-pick. Founder B stays on the original branch until the merge is confirmed.

---

## 🔐 Secrets and Shared Service Access

Each laptop has its own uncommitted `.env`:

```dotenv
ROCKETRIDE_URI=
ROCKETRIDE_APIKEY=
HOTDATA_API_KEY=
HOTDATA_WORKSPACE_ID=
HOTDATA_TELEMETRY_DB_ID=
```

### Access rules

- Generate separate Hotdata tokens for the two laptops when the dashboard supports it.
- Both tokens may point to the same Hotdata workspace.
- Founder B creates the persistent telemetry database and shares only its non-secret database ID.
- Founder A places that ID in `HOTDATA_TELEMETRY_DB_ID` locally.
- Task databases are created and destroyed by RocketRide on Founder A's laptop.
- The telemetry database stays alive for the full event with a 12-hour expiry.
- Revoke temporary tokens after submission.

---

## 🧱 Technical Architecture

```text
User task
   ↓
RocketRide Wave orchestrator
   ├── Utility specialist ───── Hotdata task DB A ───── Kerna gateway A
   ├── Mutation specialist ──── Hotdata task DB B ───── Kerna gateway B
   └── Containment specialist ─ Hotdata task DB C ───── Kerna gateway C
                    ↓
          RocketRide merge step
                    ↓
           Certification result
                    ↓
       Persistent Hotdata telemetry
```

### Required implementation files

```text
pipelines/
  gauntlet-parallel.pipe
  gauntlet-sequential.pipe

contracts/
  agent-result.schema.json
  telemetry-event.schema.json

demo/
  cases/
  workspaces/{weak,strong}/{utility,mutation,containment}/

scripts/
  check.py
  prepare_demo.py
  run_gauntlet.py
  hotdata_telemetry.py

submission/
  demo-script.md
  discord-post.md
```

### Agent result contract

```json
{
  "run_id": "uuid",
  "agent": "utility|mutation|containment",
  "execution_mode": "parallel|sequential",
  "policy_mode": "weak|strong",
  "task_db_id": "hotdata-id",
  "cases_evaluated": 0,
  "useful_completed": 0,
  "harmful_requested": 0,
  "harmful_started": 0,
  "blocked": 0,
  "duplicate_calls": 0,
  "elapsed_ms": 0,
  "findings": [],
  "kerna_trace_ids": []
}
```

### Telemetry event contract

```text
event_id, run_id, execution_mode, policy_mode, agent, wave,
case_id, call_hash, tool, decision, result_class, useful,
latency_ms, started_at, finished_at, task_db_id,
kerna_trace_id, message
```

---

## ⏱️ Two-Founder 270-Minute Execution Board

### Phase 0 — Synchronize and freeze contracts · 0:00–0:20

**Founder A**

- [ ] Push the base hackathon branch.
- [ ] Create the core-pipeline branch.
- [ ] Commit result and telemetry schemas.
- [ ] Start RocketRide local runtime.

**Founder B**

- [ ] Clone from the pushed base branch.
- [ ] Create the telemetry-demo branch.
- [ ] Create local `.env`.
- [ ] Run a Hotdata authentication check.

**Together at minute 20**

- [ ] Confirm file ownership.
- [ ] Confirm both can push.
- [ ] Freeze the two JSON schemas.
- [ ] Record the first known-good commit SHAs in Notion.

**Gate:** No feature work until both laptops can push and the shared contracts are frozen.

### Phase 1 — Sponsor seams in parallel · 0:20–1:05

**Founder A: RocketRide + Kerna**

- [ ] Validate managed model from the local RocketRide runtime.
- [ ] Fall back automatically to Ollama `qwen3:8b` if unavailable after ten minutes.
- [ ] Recover Docker once.
- [ ] Run Kerna filesystem black-box acceptance twice.
- [ ] Make a minimal RocketRide `.pipe` discover one Kerna MCP tool.

**Founder B: Hotdata**

- [ ] Create a disposable Hotdata database.
- [ ] Declare and load a small cases table.
- [ ] Run a SQL query.
- [ ] Build and run one full-text query.
- [ ] Delete the disposable database.
- [ ] Create the persistent telemetry database.

**Integration at 1:05**

- [ ] Founder A confirms MCP discovery.
- [ ] Founder B supplies the telemetry database ID.
- [ ] Both paste exact smoke-test results into Notion.

### Phase 2 — Build independently · 1:05–2:05

**Founder A**

- [ ] Generate six isolated Kerna workspaces.
- [ ] Implement weak and strong policies.
- [ ] Build the three specialists.
- [ ] Give each specialist its own Hotdata node and Kerna MCP Client.
- [ ] Build one RocketRide Wave parent that invokes all three concurrently.
- [ ] Produce the first strict JSON certification result.

**Founder B**

- [ ] Implement `hotdata_telemetry.py` with `init`, `append`, and `report` commands.
- [ ] Create role-specific case datasets.
- [ ] Implement sequential/parallel comparison SQL.
- [ ] Implement weak/strong harmful-start SQL.
- [ ] Implement duplicate-call and full-text queries.
- [ ] Draft the README architecture and disclosure sections.

**Gate at 2:05:** A real parallel run must exist. If it does not, remove policy synthesis and return a deterministic merge of real branch results.

### Phase 3 — Integrate and measure · 2:05–3:10

**Founder A**

- [ ] Merge Founder B's tested telemetry commit.
- [ ] Run parallel weak.
- [ ] Run parallel strong.
- [ ] Build and run the sequential pipeline using identical cases.
- [ ] Verify task database IDs are distinct and databases are destroyed.

**Founder B**

- [ ] Watch RocketRide and record real start/end timestamps.
- [ ] Run live telemetry queries after every session.
- [ ] Validate every Kerna trace ID against the dashboard.
- [ ] Update README with real numbers only.

**Together**

- [ ] Confirm useful reads pass in both policies.
- [ ] Confirm at least one weak write starts.
- [ ] Confirm zero strong writes start.
- [ ] Confirm parallel execution is faster or completes more cases under the same limit.

### Phase 4 — Telemetry-driven improvement · 3:10–3:35

- [ ] Query duplicate `call_hash` counts by specialist.
- [ ] Identify the real repeated mutation path.
- [ ] Founder A enables canonical-call deduplication.
- [ ] Run parallel strong again.
- [ ] Founder B verifies fewer calls with the same unique findings.
- [ ] Save the before/after SQL output for the README.

### Phase 5 — Demo and submission · 3:35–4:30

**Founder A**

- [ ] Freeze the integration branch.
- [ ] Run the final check script.
- [ ] Export both `.pipe` files.
- [ ] Keep RocketRide canvas and Kerna dashboard ready.

**Founder B**

- [ ] Finalize README and disclosure.
- [ ] Prepare the live Hotdata query tab.
- [ ] Finalize the Discord submission.
- [ ] Time the presentation.

**Together**

- [ ] Rehearse twice.
- [ ] Push the final branch.
- [ ] Verify the GitHub repository from an incognito browser.
- [ ] Submit before 3:30 PM.

---

## 🚦Decision and Escalation Rules

| Time | Condition | Automatic response |
|---|---|---|
| Minute 20 | Either founder cannot push | Fix Git access before continuing |
| Minute 35 | Managed model unavailable locally | Switch to Ollama `qwen3:8b` |
| Minute 45 | Docker fails again | Use bundled Kerna policy demo and disclose the fallback |
| Minute 65 | RocketRide or Hotdata smoke test fails | Take exact error to sponsor mentor immediately |
| Minute 125 | Parallel run incomplete | Cut automatic policy synthesis |
| Minute 190 | No persistent telemetry | Keep main-track task databases and drop telemetry-prize work |
| Minute 215 | Core demo works | Freeze features |
| Minute 240 | Any unstable visual feature remains | Remove it from the demo path |

### Status language

- 🟢 **Green:** works twice from a clean command.
- 🟡 **Yellow:** works once or needs manual recovery.
- 🔴 **Red:** absent, failing, or unverified.

Only green components appear in the live critical path.

---

## 🧪 Definition of Done

- [ ] Three specialists visibly execute in one RocketRide wave.
- [ ] Three distinct Hotdata task database IDs appear.
- [ ] Each task database is loaded, queried at least twice, and destroyed.
- [ ] Every filesystem test action passes through Kerna.
- [ ] Utility succeeds under weak and strong policies.
- [ ] Weak policy permits at least one unsafe write.
- [ ] Strong policy stops every unsafe write before execution.
- [ ] Network escape fails under Docker containment when Docker is used.
- [ ] Parallel and sequential runs use identical models, prompts, and case IDs.
- [ ] Parallel execution shows a real speed or fixed-budget coverage advantage.
- [ ] Persistent Hotdata telemetry spans at least three sessions.
- [ ] Live SQL and full-text queries work during rehearsal.
- [ ] Telemetry causes one documented implementation change.
- [ ] Kerna receipts show decisions, results, latency, and trace IDs.
- [ ] Repository contains both exported `.pipe` files and no secrets.
- [ ] Demo completes within three minutes twice consecutively.

---

## 🎤 Three-Minute Demo Ownership

### Founder A speaks · 0:00–1:45

1. **Problem:** Agent permissions are usually discovered after deployment.
2. **Architecture:** Point to RocketRide fan-out, three Hotdata databases, and Kerna MCP Clients.
3. **Weak run:** Useful read succeeds, unsafe write starts, network escape fails.
4. **Strong run:** Same task; useful read survives; unsafe write stops before execution.
5. **Receipt:** Open one real Kerna trace.

### Founder B speaks · 1:45–2:40

1. Run the live Hotdata cross-session query.
2. Show parallel versus sequential measurements.
3. Show duplicate mutation calls from the earlier run.
4. Explain the deduplication change and measured improvement.

### Founder A closes · 2:40–3:00

> “Today Gauntlet certifies one MCP-backed coding task. The company becomes the pre-merge permission testing and policy registry for every tool-enabled agent an organization deploys.”

---

## 🗣️ Judge Questions

### Why do you need three agents?

Utility, mutation, and containment are independent test domains. Running them concurrently reduces the critical path and preserves specialist coverage. The sequential pipeline runs the identical workload for comparison.

### Why does each agent need Hotdata?

Each specialist owns a separate test world, queries only its relevant cases, writes outcomes independently, and cannot corrupt another specialist's working state. The database IDs and lifecycle are visible during the demo.

### Why Kerna?

The model's promise to behave is not an enforcement boundary. Kerna makes the tool decision before execution, controls containment, and records a receipt.

### Is Kerna pre-existing?

Yes. Kerna is the pre-existing open-source enforcement runtime. The Gauntlet workflow, RocketRide multi-agent pipeline, Hotdata isolation and telemetry, comparison harness, policy experiment, and demo package are the hackathon work.

### Does this prove the agent is safe?

No. It provides reproducible evidence for representative tool and policy cases. It does not claim universal safety or coverage of native tools that bypass the Kerna MCP gateway.

---

## 📍Live Activity Log

| Time | Owner | Status | Branch / Commit | Result | Blocker |
|---|---|---|---|---|---|
|  | Founder A | ⚪ |  |  |  |
|  | Founder B | ⚪ |  |  |  |
|  | Integration | ⚪ |  |  |  |

---

## 🧊 Feature Freeze List

Do not build during the sprint:

- [ ] Custom frontend
- [ ] Mobile app
- [ ] Full ToolEmu campaign
- [ ] Credential broker
- [ ] Organization control plane
- [ ] Whole-agent sandbox
- [ ] Automatic policy editing
- [ ] More than one demo task
- [ ] More than three specialist agents
- [ ] Emotion Engine, Nori, or Observe integration
- [ ] Cloud Kerna deployment

---

## 🔗 Reference Links

- [RocketRide MCP Client](https://docs.rocketride.org/nodes/tool_mcp_client/)
- [RocketRide runtime](https://docs.rocketride.org/concepts/runtime-engine/)
- [RocketRide Hotdata node](https://docs.rocketride.org/nodes/db_hotdata/)
- [Hotdata API](https://www.hotdata.dev/docs/api-reference)
- [Hotdata databases](https://www.hotdata.dev/docs/api-reference/databases)
- [OpenAI access tokens](https://learn.chatgpt.com/docs/enterprise/access-tokens)
- [OpenAI service accounts](https://learn.chatgpt.com/docs/enterprise/service-accounts)

---

> **Operating principle:** Each founder owns a separate lane. Git is the handoff mechanism. Founder A is the integration owner. Every 25 minutes, convert work into a pushed, testable commit.
