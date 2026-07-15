# Kerna for everyday use

You don't need to be a developer. Kerna is a safe assistant that can use real tools — search the web, take notes, read pages, and (once connected) your email and calendar — and it always asks before doing anything that affects the outside world.

## Set up in 4 commands

```bash
kerna init                       # pick a model (or "Demo mode" for zero setup)
kerna pack install productivity  # adds local notes/calendar/weather plus optional web/search
kerna secrets add search         # shows how to set the free search key
kerna doctor                     # check everything is ready
```

`kerna doctor` also reports each connector as ready, needs setup, not
configured, or manifest-invalid—without printing any credential value.

## Just ask

```bash
kerna run "Search the web for the best beginner yoga routines and save the top 3 to my notes"
kerna run "Summarize this article for me: @https://example.com/post"
kerna run "What did I write in my notes about the trip budget?"
```

Kerna figures out which tools to use. When it wants to do something that touches the outside world — send a message, post something, write a file — it stops and shows you exactly what it's about to do:

```
  ⚠️  APPROVAL REQUIRED
  Tool: send_email
  This will take an external action on your behalf.
  Details:
    to: alex@example.com
    subject: Re: dinner
    body: Sounds good — see you at 7!
  Allow this action? [y/N]:
```

Nothing happens unless you type `y`. That's the whole idea: the assistant is capable, but you're always in control.

When a task is started from the desktop app, the same decision appears in its
local Approval queue instead. Each request shows one proposed tool call and
expires after five minutes. The CLI can also review or decide a queued request:

```bash
kerna approval list
kerna approval approve <request-id>
kerna approval deny <request-id>
```

## Connect Google Calendar

For a hosted calendar, install the reviewed Google connector, inspect its
permissions, and connect a Google OAuth **Desktop app** client with the
Calendar API enabled:

```bash
kerna pack install google-workspace
kerna mcp risk google-calendar
python plugins/google_calendar_mcp/connect.py --client-id "YOUR_CLIENT_ID" --save
```

The default OAuth grant can only read events. To create events, reconnect with
`--allow-write`; each creation still enters Kerna's approval queue and sends no
calendar notifications unless you explicitly ask for them. Never use a shared
or production calendar as the first test account.

## Everyday automation

Have Kerna run a routine for you on a schedule (needs the background daemon running):

```bash
kerna routine add daily-digest     # each morning: local calendar, notes, and top priorities
kerna routine add morning-news     # each morning: top AI news
kerna routine add morning-brief    # weekday brief from your approved read tools
kerna routine add meeting-prep     # weekday agenda and context for today's meetings
kerna routine add research-brief   # weekly cited research brief
kerna routine preview 0            # show its exact tool scope and policy gaps
kerna routine enable 0             # explicitly activate it after review
kerna routine run 0                # run the reviewed routine once now
kerna daemon                       # keep running to let routines fire
```

New routines start **paused**. Each template has a narrow, visible allowlist;
Kerna will only enable it after every tool on that allowlist has an explicit
`auto_approve` rule. Routines are non-interactive: Kerna never waits for
someone to answer a terminal prompt in the background. Keep writes, sends,
publishing, deletes, and payments out of routine allowlists and
approval-required.

For example, a local morning brief can be explicitly authorized with only its
read tools:

```toml
[[permissions]]
tool = "list_events"
action = "auto_approve"

[[permissions]]
tool = "list_notes"
action = "auto_approve"

[[permissions]]
tool = "search_notes"
action = "auto_approve"

[[permissions]]
tool = "get_weather"
action = "auto_approve"
```

## See what happened

Everything Kerna does is recorded. To review any run:

```bash
kerna trace last     # every step it took
kerna inspect last   # a plain summary
```

## Your privacy

- API keys and tokens live in your environment, never in Kerna's config files.
- Choose a local model (Ollama) and run `--privacy local-only` to keep everything on your machine.
- Every tool is off by default — you turn on exactly what you want.

See the [full usage guide](USING_KERNA.md) for more.
