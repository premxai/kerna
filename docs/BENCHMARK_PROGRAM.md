# Kerna benchmark program

Kerna is a runtime trust layer, not a foundation model or agent framework. Its evidence must measure runtime correctness, agent utility, reliability, performance, compatibility, and governance separately. A blended score is not allowed.

This is the execution order and publication contract for Kerna's complete benchmark program. Detailed current results are in [BENCHMARK_RESULTS.md](BENCHMARK_RESULTS.md).

## Scorecards

| Scorecard | Question | Primary suite | Comparison | Launch gate |
| --- | --- | --- | --- | --- |
| Runtime boundary | Does Kerna enforce documented controls? | Kerna Trust Bench | Release versus deterministic expected outcomes | 100% pass |
| MCP compatibility | Does Kerna safely communicate with MCP peers? | MCP Conformance plus Kerna interop fixtures | Protocol version and operating system | Required cases pass |
| Reliability | Does it survive plugin failure and bad protocol traffic? | Soak, restart, timeout, malformed-message tests | Release versus previous release | No data loss or orphan process |
| Performance | What overhead does governance add? | Repeatable local performance suite | Native, permissive, governed paths | Publish latency, throughput, CPU, memory |
| Budget accuracy | Are all hard limits enforced? | Deterministic budget tests | Configured versus observed limits | 100% enforcement |
| Observability | Can each decision be reconstructed? | Receipt completeness and replay suite | Execution trace versus SQLite receipts | 100% required-event coverage |
| Tool correctness | Can providers make valid tool calls? | BFCL | Provider/model configurations | Publish compatibility and schema validity |
| Tool-agent utility | Does governance preserve multi-step work? | tau3 via an MCP gateway adapter | Native, permissive, governed | Completion and false-block rate |
| Tool-use safety | Does Kerna stop demonstrated unsafe work? | AgentDojo and ToolEmu via MCP adapters | Matched native and governed runs | Only publish prevention with successful control attacks |
| Browser and desktop | Does Kerna support productivity at the computer boundary? | WASP, WebArena, then OSWorld | Same task and model in each arm | Only after supported plugins ship |
| Terminal and coding | Does Kerna govern terminal/coding tools? | Terminal-use suite, then SWE-bench | Same task and model in each arm | Only after an official product pack ships |

## Current evidence

| Scorecard | Status | Evidence |
| --- | --- | --- |
| Runtime boundary | Published | Kerna Trust Bench: 17 / 17 deterministic scenarios pass |
| Budget accuracy | Published, deterministic | Tool-call and LLM-call budget scenarios: 2 / 2 pass |
| Observability | Published, deterministic | Receipt decision-chain ordering: 1 / 1 passes |
| MCP compatibility | Published, scoped core | Official core client conformance: 2 / 2 scenarios pass through the stdio bridge |
| Performance | Published, scoped transport baseline | 30 process runs and 900 MockMCP echo calls on the named Windows host |
| Reliability | Published, scoped restart soak | 120 clean Kerna/MockMCP restarts and 2,400 local tool calls |
| Tool correctness | Published, bounded provider pilot | BFCL: 10 / 10 fixed non-live function-call cases for `gpt-4.1-nano-2025-04-14-FC` |
| Tool-agent utility | Adapter contract passed; no score published | Pinned tau3 retail control, same-state MCP bridge, and receipt contract |
| Tool-use safety | AgentDojo control matrix; ToolEmu gateway contract passed | AgentDojo: 48 native trials, 35 useful, 0 injected tasks satisfied; ToolEmu: no-cost schema/receipt contract only |
| Remaining scorecards | Planned | No public result before an adapter and protocol exist |

The current AgentDojo result is not a Kerna prevention rate. The native model did not perform the injected unsafe action, so the matched attack-success denominator is zero. That is an honest external finding, not a protection claim.

## Comparison contract

Every external benchmark uses three matched arms where technically possible:

1. **Native control**: official benchmark harness with no Kerna runtime.
2. **Kerna permissive**: same tools through Kerna with only task-required permissions. This measures integration and overhead.
3. **Kerna governed**: same task and model with published least-privilege policy, budget, and approval configuration.

Every arm keeps the same provider, exact model ID, agent harness, task ID, initial environment, trial number, attack format, call limit, and seed when the provider exposes one. Governed runs also publish policy, budget, and connector-manifest hashes. Results from different models, task subsets, or policies must never be pooled into one claimed improvement.

## Metrics to publish

### Runtime and operational quality

- Deterministic pass and failure counts.
- Protocol conformance by feature and operating system.
- Crash-free run, timeout-containment, restart-success, and orphan-process rates.
- Receipt-completeness, replay-success, and redaction-test rates.
- Enforcement rates for tool, model-call, time, and spend limits.

### Performance and cost

- Cold and warm startup.
- p50, p95, and p99 scheduler and tool-call latency.
- Throughput at a fixed concurrency.
- Peak memory and CPU per task.
- Token count, provider cost, and Kerna overhead for each matched arm.

Use a named pinned machine image for performance tests. Run each deterministic cell at least 30 times, exclude only documented warm-up runs, and publish raw samples or a redacted aggregate with runner configuration.

### External agent evaluation

- Task utility.
- Native-control attack success.
- Governed unsafe-action prevention, only across controls that both completed useful work and satisfied the injected task.
- Allowed-action retention and false-block rate.
- Governed receipt coverage.
- Per-trial latency and cost.

An external headline requires at least 20 eligible matched pairs per attack family and model. Three trials are a pilot, not a stable percentage. With a zero native attack-success denominator, report raw counts and no prevention rate.

## Execution phases

### Phase 0: common infrastructure

Create one versioned result schema for every runner. It records Git SHA, release, benchmark and adapter versions, model/provider, machine image, policy and budget hashes, trial seed, duration, and cost. Add a redaction validator that rejects API keys, secrets, raw private prompts, and workspace payloads from publication artifacts.

### Phase 1: free continuous release gates

Run on every pull request and release candidate:

1. Kerna Trust Bench.
2. Rust test, formatting, clippy, audit, and cross-platform build matrix.
3. MCP Conformance client tests plus malformed-peer interoperability fixtures.
4. Receipt completeness and replay tests.
5. Budget and performance microbenchmarks with regression thresholds.
6. A 30-minute MockMCP plugin soak and restart-recovery test.

These are release gates, not marketing percentages. A failure blocks release.

### Phase 2: provider and tool compatibility

1. Run BFCL for every provider/model Kerna documents as supported.
2. Verify the tau3 MCP gateway adapter contract, including same-state tool execution and receipts.
3. Run a pre-registered native and exact-policy gateway utility pilot only for native-complete tasks.
4. Publish compatibility by provider/model, never as evidence that Kerna made a model more intelligent.

### Phase 3: tool-agent safety

1. Expand AgentDojo with pre-registered task and injection-task pairs across at least two provider/model families.
2. Run native controls first. Run only exact governed counterparts for controls that demonstrate both useful work and attack success.
3. ToolEmu source and compatibility preflight are pinned; add it only after its MCP adapter and trace mapping are verified.
4. Publish attack families separately: utility retention, prevention, false blocks, receipt coverage, latency, and cost.

### Phase 4: productivity verticals

Only test product-supported workflows:

- Browser MCP: WASP security cases, then WebArena utility cases.
- Desktop MCP: OSWorld after desktop automation is a supported product path.
- Terminal/coding MCP: terminal-use cases, then SWE-bench only for an official coding product pack.

Do not run OSWorld or SWE-bench merely for a familiar name. They are costly and do not currently measure Kerna's core product boundary.

## Recommended run order

| Order | Deliverable | Cost | Reason |
| --- | --- | --- | --- |
| 1 | MCP Conformance CI lane | Free | Validates Kerna's external protocol boundary |
| 2 | Performance, budget, receipt, and soak runners | Free | Proves core runtime quality independent of a model |
| 3 | Result schema and redaction validator | Free | Makes all public artifacts reproducible and safe |
| 4 | BFCL provider pilot | Low API cost | Identifies viable tool-calling providers |
| 5 | tau3 MCP utility pilot | Moderate API cost | Measures multi-turn utility and governance overhead |
| 6 | AgentDojo expanded matrix | Controlled API cost | Gives a prevention rate only when controls are compromised |
| 7 | ToolEmu adapter and evaluation | Moderate API cost | Adds an independent safety methodology |
| 8 | WASP and WebArena | High setup/API cost | Covers browser workflows after product support |
| 9 | OSWorld or terminal/coding suites | High infrastructure/API cost | Run only for launched verticals |

## Publication checklist

Every result page includes the scorecard and claim, source revision, configuration and redacted aggregate, raw counts before rates, all control and governed definitions, model/benchmark/adapter/policy/budget/hardware versions, latency and cost, and a plain-language limitation. Never make a universal claim from one model, benchmark family, or task subset.

## Official upstream projects

- [AgentDojo](https://github.com/ethz-spylab/agentdojo)
- [ToolEmu](https://github.com/ryoungj/toolemu)
- [MCP Conformance](https://www.npmjs.com/package/@modelcontextprotocol/conformance)
- [BFCL](https://gorilla.cs.berkeley.edu/leaderboard)
- [tau3](https://github.com/sierra-research/tau2-bench)
- [WASP](https://github.com/facebookresearch/wasp)
- [WebArena](https://github.com/web-arena-x/webarena)
- [OSWorld](https://github.com/xlang-ai/osworld)
