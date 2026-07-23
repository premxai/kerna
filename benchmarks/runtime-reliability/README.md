# Kerna runtime reliability soak

This runner repeatedly starts a fresh Kerna process. Each process starts the
built-in MockMCP child, completes MCP initialization and tool discovery, makes
a fixed number of `echo` calls, and exits.

Run a local publication-grade soak:

    node benchmarks/runtime-reliability/run.mjs --runs 120 --iterations 20 --out reports/runtime-reliability/latest.json

A passing result proves all configured iterations exited successfully and
completed their expected local tool calls. It complements the deterministic
timeout and malformed-protocol scenarios in Kerna Trust Bench.

It does not measure model reliability, provider availability, scheduler work,
or a general operating-system orphan-process guarantee. Those need separate
product-level tests before being claimed.
