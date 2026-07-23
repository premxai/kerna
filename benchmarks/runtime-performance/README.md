# Kerna runtime performance

This deterministic runner measures the core process-isolated stdio MCP path with
the built-in MockMCP `echo` tool. It records:

- child-process spawn plus MCP initialization;
- `tools/list` discovery; and
- repeated `tools/call` echo latency.

It deliberately excludes the scheduler, SQLite, provider calls, network
connectivity, and model latency. Those are separate scorecards and must not be
mixed into these transport metrics.

Run a local publication-grade sample:

    node benchmarks/runtime-performance/run.mjs --runs 30 --iterations 30 --out reports/runtime-performance/latest.json

The runner records the operating system, CPU model, sample count, source
revision, and p50/p95/p99 values. CI uses a smaller smoke sample to catch a
broken benchmark path; it does not enforce cross-machine latency thresholds.
