# Kerna for everyday use

You don't need to be a developer. Kerna is a safe assistant that can use real tools — search the web, take notes, read pages, and (once connected) your email and calendar — and it always asks before doing anything that affects the outside world.

## Set up in 4 commands

```bash
kerna init                       # pick a model (or "Demo mode" for zero setup)
kerna pack install productivity  # adds web search, notes, and web reading
kerna secrets add search         # shows how to set the free search key
kerna doctor                     # check everything is ready
```

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

## Everyday automation

Have Kerna run a routine for you on a schedule (needs the background daemon running):

```bash
kerna routine add daily-digest     # each morning: unread email + calendar + top priorities
kerna routine add morning-news     # each morning: top AI news
kerna routine list
kerna daemon                       # keep running to let routines fire
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
