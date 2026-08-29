# Kerna Observe — measure what your coding agents cost

Your engineers are running AI coding agents. You are paying for every turn, and the bill
is the only thing you can see. Kerna Observe sits between those agents and your provider
and answers two questions with your own traffic:

- **Where does the money actually go** — which models, which kinds of work, how much of
  it is the same context sent again.
- **How much of it could have run on the laptop it was typed on** — measured rather than
  estimated, by attempting the same turn locally and comparing it against the answer your
  provider already gave. This half is optional and needs a local model; the spend half
  needs nothing.

It takes about a week of ordinary work to say anything useful, and it costs one file.

## See it first, in one command

```
kerna-observe demo
```

Stands the whole system up against a stub provider and opens the report. No API key, no
account, no network, nothing installed. Every number in that report is synthetic and the
page says so — it shows you the shape of the answer, not an answer.

---

## What it does not do

Read this part first. It is the reason this is safe to install on a working machine.

**It never answers a request.** Every request is forwarded to your provider, unchanged,
and their answer is what your agent receives. Local attempts happen afterwards, on an
idle machine, and the result is thrown away after being compared. Nothing your engineers
see comes from us.

**It never holds your API key.** Your agent's `Authorization` header is passed straight
through and is not stored, logged, or copied. An install requires no credential from us
and leaks none if the machine is compromised — there is nothing here to take.

**It does not record your code or your prompts.** The logs contain structure and counts:
how many messages, which model, how many tokens, which tools were offered. Not the
content of any of them.

**Removing it is one environment variable.** There is no uninstaller, no service, and no
agent left behind.

---

## 1. Install

Download `kerna-observe` and put it anywhere on your PATH. It carries its own runtime —
there is nothing to install first, no Python, no dependencies.

> **Today there is a Windows build only.** macOS and Linux are supported by the code and
> have their own data directories, but no binary is published for them yet; on those
> platforms build from source (`packaging/build.py`) until one is. Tell us which platform
> you are on — that is more useful to us than a guess.

```
kerna-observe install
```

That prints what to set, and what to unset to remove it. It does not change anything
itself: putting a process in the path of your production traffic is your decision to
make, not something a tool should do to you while you read its output.

## 2. Point your agent at it

In the shell where your agent runs:

```
OPENAI_BASE_URL=http://127.0.0.1:8127/v1
ANTHROPIC_BASE_URL=http://127.0.0.1:8127
```

Set whichever your agent uses. Both dialects are supported — OpenAI-compatible clients
and Anthropic clients such as Claude Code.

## 3. Run it

```
kerna-observe run --upstream https://api.anthropic.com/v1
```

`--upstream` is your provider's normal endpoint — the one your agent would have called
without us. Use `https://api.openai.com/v1` for OpenAI.

It prints where it is writing before it does anything else:

```
cascade sidecar on http://127.0.0.1:8127  ->  https://api.anthropic.com/v1
  local routing DISABLED — every request goes to the cloud, unchanged. [...]
  traffic:   C:\Users\you\AppData\Local\Kerna\traffic.jsonl
  install:   OPENAI_BASE_URL=http://127.0.0.1:8127/v1
  uninstall: unset OPENAI_BASE_URL
```

(The routing line is longer than shown; it carries the measured reason routing is off.)

Then work normally for a week. It does nothing you will notice.

## 3b. Optional: compare against a local model

Everything above measures spend. To also answer *"could this have run locally?"*, point
the sidecar at a local model server — llama.cpp, Ollama, or anything OpenAI-compatible:

```
kerna-observe run --upstream https://api.anthropic.com/v1 --explore http://127.0.0.1:8080
```

Local attempts run **only while nobody is working** — no request served for the last 20
seconds — so they never compete with your engineers for the GPU. Nothing they produce is
ever sent to anyone; it is compared against your provider's answer and discarded.

Without `--explore` the report's comparison panels simply say nothing has been compared,
which is the honest state rather than an empty chart.

## 4. Read the report

```
kerna-observe report
```

That writes one self-contained HTML file and prints its path. Open it in a browser. It
needs no network and loads nothing from anywhere — it opens on a disconnected laptop.

If you also run the Kerna runtime, add its audit trail and the report joins what policy
decided to what the turn cost:

```
kerna-observe report --kerna-db path/to/kerna.db
```

A denial recorded under `kerna run --audit` is counted separately and labelled, because
audit mode records the decision and lets the action run — reporting the two together
would make an unenforced policy look like protection.

## 5. Remove it

Unset the environment variables and stop the process. That is the whole uninstall.

Your evidence stays where it was written; delete that directory if you want it gone.

---

## Where things are written

| Platform | Directory |
|---|---|
| Windows | `%LOCALAPPDATA%\Kerna` |
| macOS | `~/Library/Application Support/Kerna` |
| Linux | `$XDG_DATA_HOME/kerna`, else `~/.local/share/kerna` |

Override with `--traffic-log` and `--explore-log`. The resolved paths are always printed
at startup, so what you are reading and what is being written cannot drift apart without
you seeing it.

---

## What the report will and will not tell you

**It will tell you** what you spent, on which models, split by the kind of work; how much
of your spend is agent turns rather than one-shot questions; and how much of your context
is tool definitions rather than conversation — which is usually the surprise.

**It will not tell you that local models are as good as your provider's.** Where a local
comparison exists the report gives the rate with its confidence interval, and turns that
could not be compared fairly are excluded and counted separately rather than folded in.
Every panel that has no data says so instead of showing a zero.

**Local routing is off.** Nothing is answered locally by default, in this release or on
your machine. The measurement is the product; routing is a decision you would make later,
with the numbers in front of you.

---

## If something goes wrong

**`cannot listen on 127.0.0.1:8127`** — something already has that port, most often a
sidecar you left running. Stop it, or pass `--port`. It refuses to start rather than
sharing a port, because two sidecars on one port would split your evidence across two
files and neither would say so.

**Your agent cannot connect** — check the sidecar is running and the variable is set in
the *same* shell the agent starts from. Many editors do not inherit a variable you
exported in a terminal after the editor launched.

**Requests fail that used to work** — unset the environment variable. Your agent goes
straight back to your provider immediately, with no other change. If that fixes it, the
problem is ours; the log directory above has what we need to diagnose it.

**Nothing appears in the report** — check the traffic log path printed at startup is the
one `report` is reading. Pass `--traffic` explicitly if you moved it.
