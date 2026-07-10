# Kerna Go-To-Market & Traction Plan

## Positioning

**Kerna is the seatbelt and black-box recorder for AI agents.**

It does not compete with agent frameworks — it runs *underneath* them. The one-line pitch:

> Run OpenClaw, Hermes, Claude Code, or any MCP tool stack **on top of Kerna** and get budgets, fail-closed permissions, and a full forensic trace for free.

Every other agent project is racing to be the smartest agent. Kerna owns a different, defensible axis: **trust and observability**. That's the axis enterprises and security-conscious developers actually block on.

## The wedge: lead with the demo, not the architecture

The single most shareable moment is an agent trying `rm -rf /` and Kerna denying it, followed by `kerna trace last` showing the complete forensic record. Lead every channel with that 15-second GIF. Architecture talk converts nobody; a blocked catastrophe converts everybody.

## Highest-leverage ideas (ranked)

1. **MCP policy-gateway mode** — expose Kerna *itself* as an MCP server that proxies other MCP servers through its policy engine. This makes Kerna instantly useful to every Claude Code / Cursor / Cline user **without adopting a new runtime** — they point their existing client at Kerna and gain policy + audit. This is the biggest single unlock and a natural v0.3 headline.

2. **Zero-key demo mode (`kerna demo`)** — a guided tour on the mock provider + MockMCP. Removes the #1 funnel drop-off (needing an API key before you can see value). Ship it as the literal first command in the README (already the case via `kerna run "Please call echo"`).

3. **`kerna trace --html`** — export a task trace as a single self-contained HTML file. Every shared trace becomes marketing, and it's the natural artifact to attach to a bug report or an audit.

4. **Ollama-first quickstart** — a fully local, fully private agent in three commands. The local-LLM community (r/LocalLLaMA and friends) is the most eager early-adopter pool, and *no* major agent runtime currently leads with **trust + local**. Kerna can own that intersection.

5. **Preset provider ecosystem** — the 10 built-in providers mean "works with whatever model you already pay for" is true on day one. Keep the list current; each new preset is a keyword people search for.

## Launch sequence

1. Land the README + killer-demo GIF, tag **v0.2.0**, publish prebuilt binaries via the release workflow.
2. Show HN with the blocked-`rm -rf` GIF as the lead image; title around "trust layer / black box for agents," not "another agent framework."
3. Post to r/LocalLLaMA (Ollama-first angle) and the MCP community Discord (policy-gateway angle).
4. Follow up within a week with the `trace --html` feature and one real integration writeup ("running <framework> under Kerna").

## Proof points to have ready at launch

- All quality gates green (see `LAUNCH_READINESS_REPORT.md`).
- The blocked-command demo reproducible from a clean `kerna init`.
- One real end-to-end run per tier: an OpenAI run, an Anthropic multi-turn tool-use run, and an Ollama local run.
