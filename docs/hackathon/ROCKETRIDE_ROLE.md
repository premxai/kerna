# RocketRide's role in Kerna Gauntlet

## What it is

RocketRide is the **visual AI pipeline builder, orchestration engine, and deployment runtime** for the project. The VS Code extension authors the workflow on a canvas, the `.pipe` file stores the version-controlled graph, the multithreaded engine executes it, and the Python SDK lets our demo shell call it.

The product UI remains a thin Kerna demo surface. The agent behavior lives in RocketRide.

## Exact pipeline

```text
Chat input
    ↓
Coordinator: RocketRide Wave
    ├── invokes Utility specialist ───── Hotdata A + Kerna MCP client A
    ├── invokes Mutation specialist ──── Hotdata B + Kerna MCP client B
    └── invokes Containment specialist ─ Hotdata C + Kerna MCP client C
                  [one parallel tool-call wave]
    ↓
Coordinator validates and merges three strict JSON results
    ↓
Permission certificate
    ↓
Python SDK returns the certificate to the demo shell
```

## Why the graph is credible

- The coordinator is an `agent_rocketride` node with an LLM and `memory_internal` connection.
- Each specialist is another agent node exposed to the coordinator through its tool control connection.
- The coordinator is instructed to invoke all three specialists in the same wave.
- Each specialist controls a separate `db_hotdata` node, creating a distinct ephemeral database for that pipeline run.
- Each specialist controls a `tool_mcp_client` node, which discovers and calls real Kerna MCP tools.
- `require_tool_call` is enabled so a specialist cannot return a plausible narrative without using a real external tool.
- The parent emits one certificate only after all three specialist results have arrived and passed schema checks.

## What we build around it

| Layer | Implementation |
|---|---|
| Pipeline authoring | RocketRide VS Code visual canvas |
| Portable workflow | `pipelines/gauntlet-parallel.pipe` |
| Sequential benchmark | `pipelines/gauntlet-sequential.pipe` |
| Execution | RocketRide local for the first complete proof; RocketRide Cloud after the Kerna HTTP seam works |
| Application call | `scripts/run_gauntlet.py` using the RocketRide Python SDK |
| Product surface | Small Kerna result view or CLI showing the certificate and trace IDs |
| Enforcement | Kerna reached through MCP Client `stdio` locally or Streamable HTTP from Cloud |
| Per-agent data | Separate Hotdata nodes controlled by each specialist |

## Build order

1. Build `Chat → RocketRide Wave → Answers` and run it.
2. Add one specialist as a tool and prove a real invocation.
3. Add that specialist's Hotdata node and prove create, load, query, and cleanup.
4. Add its Kerna MCP Client and prove `tools/list` and one real tool call.
5. Duplicate the proven specialist pattern for mutation and containment.
6. Make the coordinator invoke all three in one wave and merge their JSON.
7. Expose the local Kerna gateway through a temporary authenticated Streamable HTTP adapter that RocketRide Cloud can reach.
8. Switch the Cloud MCP Client profile from local `stdio` to that HTTP endpoint.
9. Export the parallel `.pipe`, clone it into a sequential benchmark, and keep inputs identical.
10. Deploy the final pipeline to RocketRide Cloud.
11. Call the deployed pipeline from `run_gauntlet.py` and rehearse from one command.

Do not start with a custom dashboard. The first milestone is a real three-agent Cloud pipeline with evidence. Add a thin result view only after that milestone works twice.

## What judges should see

1. The RocketRide canvas with the coordinator, three specialists, three Hotdata nodes, and Kerna MCP connections.
2. A Cloud execution trace showing the three specialist calls within the same wave.
3. Three distinct Hotdata task database IDs.
4. Kerna receipts proving allow and deny decisions happened before tool execution.
5. Both submitted `.pipe` files in GitHub and the RocketRide showcase post.

## Cloud connectivity boundary

The MCP Client's `stdio` mode starts its command beside the RocketRide engine. It can launch the local `kerna gateway` binary during local development. A Cloud engine cannot see the binary or `localhost` on Founder A's laptop.

For the end-to-end Cloud run, the MCP Client must use `streamable-http` with an authenticated, temporarily reachable Kerna gateway adapter. Keep the bearer token only in RocketRide's encrypted configuration and close the adapter after judging.

Prove local `stdio` first. Spend at most fifteen focused minutes on the HTTP adapter and public tunnel before asking a RocketRide mentor for the supported Cloud pattern. If that seam remains blocked, show the complete enforcement path locally and a real Cloud deployment of the same RocketRide/Hotdata graph, and disclose that transport boundary precisely.

## Ten-minute fallback rule

Develop against RocketRide Cloud first because the event's RocketRide prize rewards Cloud deployment and tracks platform usage. If Cloud access or credits remain blocked after ten focused minutes plus sponsor escalation, run the same `.pipe` locally and disclose the fallback. Do not redesign the product around the outage.
