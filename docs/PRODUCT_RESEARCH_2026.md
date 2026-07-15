# Kerna Product Research and Daily-Use Roadmap

Updated: 2026-07-15

## Executive conclusion

Kerna should become the **trusted operating layer for useful, recurring AI
work**: a local-first runtime that lets a person or team connect their real
tools, automate bounded jobs, approve consequential actions, and recover the
complete receipt of what happened.

The market does not need another generic chat agent or another catalog of thin
integrations. MCP is increasingly the interoperability layer used by agent
products, while users still struggle with: deciding what an agent may touch,
connecting tools safely, reviewing an action before it happens, and proving
what happened afterward. Those are Kerna's natural strengths.

The product must therefore sell an outcome, not a component:

> "Your daily AI operator, with a seatbelt, a permission wallet, and a receipt
> for every action."

Kerna should continue to own governance, orchestration, memory, and
observability. Email, calendar, CRM, task manager, browser, document, and
communication behavior must remain MCP plugins/connectors rather than becoming
domain logic in the kernel.

## Evidence from the current ecosystem

- MCP is no longer a niche local-desktop convention. Anthropic reported more
  than 10,000 active public MCP servers and adoption across ChatGPT, Cursor,
  Gemini, Microsoft Copilot, and VS Code. The protocol is now under the Linux
  Foundation's Agentic AI Foundation. [Anthropic, Dec. 2025](https://www.anthropic.com/news/donating-the-model-context-protocol-and-establishing-of-the-agentic-ai-foundation?content=Dec2024EOYShips&medium=email&messageTypeId=140367&source=i_email)
- Major AI products are making connectors central to daily work. OpenAI lists
  Gmail, Calendar, Contacts, Outlook, Teams, Drive, SharePoint, GitHub, and
  other sources as inputs for meeting prep, follow-ups, searching company
  knowledge, reporting, and coding. [OpenAI](https://openai.com/index/more-ways-to-work-with-your-team/)
- The MCP specification explicitly recommends clear tool visibility and a
  human ability to deny tool invocations. This validates Kerna's approval and
  trace model as a product feature, not background plumbing.
  [MCP Tools specification](https://modelcontextprotocol.io/specification/2025-06-18/server/tools)
- Remote MCP means OAuth, resource-specific tokens, and token-audience
  validation are now table stakes. A stdio-only, environment-secret workflow
  will not be enough for normal SaaS use.
  [MCP Authorization specification](https://modelcontextprotocol.io/specification/2025-11-25/basic/authorization)
- Notion's MCP positioning reinforces the most valuable everyday patterns:
  research organization, meeting/action-item conversion, product planning,
  editorial work, and personal planning. Its guidance also reinforces a
  practical launch rule: start with single-player workflows and preserve the
  source system's permissions. [Notion MCP](https://www.notion.com/help/notion-mcp)
- Zapier demonstrates the integration breadth users will expect (thousands of
  apps and tens of thousands of actions). Kerna should not rebuild that breadth
  itself; it should provide the superior governance layer and optionally broker
  a small number of trusted automation connectors. [Zapier MCP](https://zapier.com/blog/zapier-mcp-guide/?trk=public_post_comment-text)
- Tool metadata is only a hint. The MCP community is explicit that annotations
  such as `readOnlyHint` cannot be trusted by themselves; policy must be
  enforced by the host. This is directly aligned with Kerna's fail-closed
  posture. [MCP tool-annotation analysis](https://blog.modelcontextprotocol.io/posts/2026-03-16-tool-annotations/)

## Product thesis and positioning

### The primary user

Start with an individual knowledge worker who is technical enough to install a
desktop/CLI tool but does not want to hand-build automations:

- developer, founder, product manager, researcher, consultant, or operations
  lead;
- uses email, calendar, documents, notes, browser research, Slack/Teams, and
  tasks every day;
- wants a capable assistant but does not want it silently sending messages,
  altering files, or retaining unreviewed memories.

The first team buyer is a security-conscious small company that permits
AI-assisted work but needs shared policy, connector review, and evidence for
each run. Do not start with a generalized enterprise platform; prove retention
with useful personal routines first.

### The wedge

"Safe recurring work across the tools I already use."

Kerna's differentiators should be visible in the user experience:

1. **One control plane for any model and MCP tool.** The user is not locked to
   one chatbot or one vendor.
2. **Permission wallet.** Each connected tool has scopes, expiry, owner,
   last-use time, approval mode, and a one-click revoke action.
3. **Approval inbox.** Consequential work becomes readable proposed actions:
   who will receive what, what data leaves the device, what changes, and what
   will be billed.
4. **Action receipts.** Every completed routine produces a short human result,
   source links/artifacts, actions taken, approvals, spend, and a trace link.
5. **Local-first trust.** Local SQLite history, BYOK, local model routing, and
   explicit folder grants are meaningful choices for sensitive work.

## Jobs to be done: the daily-use portfolio

Build a small set of repeatable "jobs" that users can run manually first and
turn into routines only after trust is earned. All names below are product
templates; implementation belongs in plugins/packs plus generic kernel
capabilities.

| Job | Trigger | Reads | Proposed writes | Default policy | Success signal |
|---|---|---|---|---|---|
| Morning brief | weekday 8:00 | calendar, starred email, task list, selected news | optional note | reads auto/run; writes need approval | user opens one concise brief daily |
| Meeting prep | 30 min before meeting | event, attendees, recent email/docs/tasks | agenda draft | read-only, draft only | fewer manual tabs before meetings |
| Follow-up assistant | after a meeting or on demand | transcript/notes, email, tasks | email drafts, task drafts | never send/create without approval | accepted drafts per meeting |
| Inbox triage | daily/on demand | email | labels/archive/drafts | start read-only; batch changes approved | messages processed with low correction rate |
| Daily planner | each morning | calendar, tasks, priorities/preferences | task plan/note | proposal-only | plan accepted or edited quickly |
| Research brief | on demand/weekly | web, saved sources, notes | cited Markdown/Doc/Notion draft | sources and export approval visible | shareable brief with citations |
| Project pulse | weekday | GitHub/Linear/Jira/Slack/docs | status draft, risk list | draft-only | team update needs little rewriting |
| Knowledge capture | on demand | clipboard/file/voice/transcript | note with tags | confirmation for write | notes are retrievable later |
| File clean-up | on demand | selected folder | move/rename/delete plan | dry-run first; each batch approved | no accidental loss; clear undo receipt |
| Personal admin | weekly | calendar, email, documents | reminders/checklists | approval for external actions | recurring chores completed reliably |

The first three launch routines should be: **Morning Brief, Meeting Prep, and
Research Brief**. They create value with mostly read access and build trust
before Kerna asks for write/send permissions.

## Connector and tool strategy

### Tier 1: daily-use connectors

Prioritize remote OAuth MCP connectors or carefully wrapped official servers
for:

1. Google: Gmail, Calendar, Drive, Contacts.
2. Microsoft: Outlook mail/calendar, OneDrive/SharePoint, Teams.
3. Knowledge and notes: Notion, Obsidian/local Markdown, Google Docs.
4. Work tracking: Linear, GitHub, Jira, Asana/Todoist.
5. Communication: Slack and Teams.
6. Research: browser/search, RSS, web clipping, PDFs/documents.

The UI must expose a connector by *job capability* ("prepare meetings") in
addition to technical name ("Google Calendar MCP"). A user should see exactly
which scopes and tools are being granted.

### Tier 2: productivity primitives

Keep and strengthen the existing simple plugins:

- files: read/list/search/write/move with named roots, previews, diff, and
  reversible move/restore;
- notes: Markdown-backed knowledge capture and semantic retrieval;
- web/search: citations, domains, freshness, page snapshots, injection labels;
- email/calendar: OAuth-based production replacements for the app-password
  email and local-ICS prototypes;
- documents: explicit artifact generation (Markdown first, then Docx/PDF/CSV);
- tasks: normalized create/update/list interface backed by separate MCP
  plugins.

### Tier 3: do not build in core

Do not add a native CRM, project manager, browser automation product, email
client, cloud sync service, or a proprietary plugin framework to the kernel.
Those should be MCP integrations, manifest conventions, curated packs, and
examples. Kerna's unique work is enforcing the boundary consistently.

## Required product capabilities

### 1. First-class remote MCP and OAuth

This is the largest product gap for everyday SaaS work.

- Support Streamable HTTP/remote MCP, OAuth discovery, authorization-code flow,
  refresh/revocation, resource/audience binding, and scoped credential storage.
- Keep tokens outside LLM context and event payloads. Store references and
  redacted metadata only.
- Use progressive scope requests: connect read access first; request a write
  scope at the moment it is needed.
- Add connector health: last successful call, token expiry, required reauth,
  rate-limit status, and an explicit disconnect/revoke workflow.

### 2. Better policy than tool-name matching

Kerna should be the best place to answer "may this particular action happen?"

- Introduce structured action classes: read, create, modify, delete, send,
  publish, spend, credential, network-egress, and execute.
- Evaluate policy from: connector identity, OAuth scope, tool schema/metadata,
  arguments, source data sensitivity, destination, user mode, and routine
  policy—not only a tool name.
- Implement taint-aware escalation: content from web/email/untrusted MCP tools
  makes later exfiltration or external writes require an additional approval.
- Treat server-provided risk annotations as untrusted input unless supplied by
  a reviewed/verified connector.
- Add time-bound and quota-bound grants: "allow 3 GitHub issue drafts today"
  or "permit write to this folder for this run."

### 3. An approval experience people will use

Terminal prompts alone cannot make Kerna a daily tool.

- One approval queue across CLI, desktop, and mobile-friendly local web UI.
- A card explains *effect*, not JSON: recipients, account, changed fields,
  attached files, sources, external destination, estimated cost, and undo
  option.
- Support approve once, approve this exact batch, deny, edit-and-approve, and
  always ask. Never offer a casual "approve all forever" shortcut.
- For a routine, show a dry-run/preview before first enabling it.
- Generate an action receipt after every run; make it easy to share/export.

### 4. Routines and reliability

Routines should make Kerna habitual instead of novelty software.

- Define routines as versioned declarative plans: trigger, eligible tools,
  policy, budget, model/privacy route, output destination, and notification.
- Make scheduled runs idempotent; record a run key, retry class, backoff,
  timeout, and final state.
- Provide a routine gallery with transparent templates rather than opaque
  prebuilt agents.
- Send failure and approval notifications; never silently retry a
  side-effecting action without an idempotency key.
- Add a "pause all automations" control and routine-level kill switch.

### 5. Useful memory without surveillance

- Separate explicit user preferences, approved durable facts, task history,
  connector indexes, and transient run context.
- Surface what was recalled and let the user delete/forget it from the receipt.
- Require review before new durable memories become global behavior.
- Support per-workspace and per-connector retention policies; default to local
  storage with clear export/delete controls.

## Current-repository implications

The existing kernel already provides a strong base: fail-closed permissions,
budget enforcement, SQLite events, local semantic recall, folder grants,
provider routing, a plugin registry/packs, scheduled jobs, and an MCP gateway.

The work that should happen before broad daily-use launch is:

1. Make plugin manifests enforceable, not only inspectable. The current
   registry loads manifests but notes that manifest capabilities are not yet
   merged into enforcement.
2. Define the actual native-plugin isolation promise. A sandbox working
   directory is valuable, but it is not equivalent to a hardened OS sandbox.
   Default policy and UI copy must be precise; offer hardened execution modes
   where possible.
3. Replace environment/app-password productivity integrations with OAuth remote
   connectors for email, calendar, documents, and collaboration tools.
4. Build the approval inbox and action-receipt UI. The current Tauri project is
   a stale prototype with hard-coded paths and should be replaced or removed
   before being presented as product.
5. Consolidate versioning, roadmap, changelog, docs, and release state. The
   product currently has more functionality than its v0.1 messaging reflects.

## Prioritized delivery plan

### Phase 1 — make the existing value trustworthy and visible (0–6 weeks)

- Fix the desktop/control-surface direction: a small local web/Tauri control
  plane for tasks, approvals, receipts, connected tools, and routines.
- Enforce manifest capability contracts; add a reviewed-connector trust store.
- Redact secrets and sensitive tool arguments from traces by default, with
  field-level redaction rules.
- Publish one complete, zero-key demo and three manual daily-use packs:
  research, developer/project pulse, and local knowledge capture.
- Add explicit product telemetry only with user consent; initially measure
  local aggregate counters rather than contents.

### Phase 2 — connect the daily tool stack (6–14 weeks)

- Remote MCP + OAuth foundation.
- Official/curated connectors for Google Workspace, Microsoft 365, Notion,
  Slack/Teams, GitHub, and one task system (Linear or Todoist).
- Implement Morning Brief, Meeting Prep, Research Brief, and Follow-up Draft
  templates as versioned routines.
- Add approval batching, diff/previews, artifact delivery, connector health,
  and revoke controls.

### Phase 3 — team governance and distribution (14–24 weeks)

- Shared but scoped workspace policy, reviewed connector catalog, audit export,
  and admin-controlled templates.
- Gateway deployment guidance for Claude Code, Cursor, ChatGPT custom MCP, and
  other MCP-capable clients.
- Policy simulation for real routines, not merely individual tools.
- Marketplace/directory metadata, connector quality tests, and compatibility
  certification.

## Product metrics

Track outcomes, not raw model calls:

- activation: a user connects one tool and receives one useful receipt within
  15 minutes;
- weekly retained users: run at least two routines in two different weeks;
- time-to-value: median time from trigger to a usable brief/draft;
- trust: approval acceptance rate, denial rate, edit-before-approval rate,
  and repeat use after a denial;
- reliability: routine success rate, p95 duration, connector auth failure rate,
  recovery rate, and idempotent retry rate;
- safety: policy blocks, escalation after untrusted input, redaction coverage,
  and zero unapproved external side effects;
- quality: user marks output useful, artifact reuse/share rate, and manual
  correction required per routine.

Never optimize for total tool calls or autonomous minutes. Those can increase
cost and risk without increasing user value.

## Launch guardrails

- Start with read/summarize/draft workflows; earn the right to enable writes.
- Make external sends, deletes, purchases, publishing, credential operations,
  and broad data export always require clear approval.
- Require connector provenance and maintain a reviewed catalog; do not imply
  that an arbitrary MCP manifest is a security guarantee.
- Provide a plain-English data-flow view: source, model/provider, destination,
  retention, and permission used.
- Test every curated connector against malformed output, prompt injection,
  OAuth expiry, partial failure, duplicate execution, rate limits, and revoke.
- Keep the kernel narrowly scoped. Feature requests that implement a business
  action belong in plugins.

## Source notes

This brief distinguishes observed market evidence from recommendations. The
roadmap and prioritization are product inferences based on Kerna's architecture
and the cited sources, not claims made by those sources. Sources were checked
on 2026-07-15. Key primary references:

- [MCP authorization](https://modelcontextprotocol.io/specification/2025-11-25/basic/authorization)
- [MCP tool behavior and human control](https://modelcontextprotocol.io/specification/2025-06-18/server/tools)
- [MCP client best practices](https://modelcontextprotocol.io/docs/develop/clients/client-best-practices)
- [MCP tool annotations are untrusted hints](https://blog.modelcontextprotocol.io/posts/2026-03-16-tool-annotations/)
- [Anthropic: MCP ecosystem and governance](https://www.anthropic.com/news/donating-the-model-context-protocol-and-establishing-of-the-agentic-ai-foundation?content=Dec2024EOYShips&medium=email&messageTypeId=140367&source=i_email)
- [Notion MCP use cases and controls](https://www.notion.com/help/notion-mcp)
- [OpenAI: connectors for everyday team work](https://openai.com/index/more-ways-to-work-with-your-team/)
- [OpenAI: apps built on MCP](https://openai.com/index/introducing-apps-in-chatgpt/)
- [Zapier MCP scope and access model](https://zapier.com/blog/zapier-mcp-guide/?trk=public_post_comment-text)
