# kerna-observe — the model seam

Kerna governs what an agent is allowed to **do**. This half sits one layer out, between
the agent and its provider, and answers what that work **costs** — and whether any of it
could have run for free on the machine it was typed on.

It never answers a request. Every response comes from your provider, unchanged.

```bash
python -m observe.cascade.showcase
```

No API key, no provider account, no local model, no network. It stands the whole system
up against a stub provider and opens a report.

## What it does

**Records what each turn costs.** Which models, which kinds of work, and how much of the
bill is the same context sent again. On real traffic that last number is usually the
finding: most tokens are *carrying context*, not generating answers.

**Attempts the same turn locally, afterwards.** Point `--explore` at any
OpenAI-compatible server and eligible turns are replayed against it **only while nobody
is working** — no request served for 20 seconds — and the result is compared against the
answer your provider already gave, then discarded.

**Decides what has earned the right to run locally.** Agreement accumulates per task
class, per machine tier, per model. A class is promoted only when its Wilson lower bound
clears the bar with at least 30 scoreable trials. Promotion authorises *attempting*
locally; the answer is still checked and escalation is still free.

**Governs the same seam.** A tool call is visible in the model's response before the
client executes it, so one process sees both what an agent proposes to do and what it
costs — one log, one correlation id.

## The rule everything else follows

**A served request must be untouchable.** Not "unlikely to be affected" — structurally
incapable of being affected. Three consequences, each a thing this code refuses to do
rather than does carefully:

- **Submitting never blocks.** A full queue drops the item and counts the drop.
- **The worker cannot raise into anything.** A local model server dying mid-attempt is an
  ordinary Tuesday.
- **Work happens only when the developer is not working.** Evidence is paid for with
  electrons, never with anyone's time.

## Local routing is off by default

`--serve-local` exists and is not the default. On a matched fairness test the local model
slipped **8.1%** of wrong answers past a correctness gate against the cloud's **3.8%** —
a gap of +4.3 points with a 95% interval of `[-1.2, +9.8]`. That interval contains zero,
so it is not a kill; it also contains the kill threshold, so it is not proof of parity. A
shipped default cannot rest on "probably fine".

Two switches must agree before anything is served locally, and neither substitutes for
the other: the operator passes `--serve-local`, **and** a task class has earned promotion
from evidence gathered on that machine with that model.

## What it cannot tell you

Printed in the report itself, for the same reason it is here.

- **Agreement is not correctness.** Local answers are compared against the cloud's own
  answer, which itself ships a wrong answer past a test gate about 4% of the time. Two
  models can agree and both be wrong.
- **Nothing was served locally.** Every answer came from your provider. This measures what
  *could* have been, at no risk.
- **A saving needs a machine that can run the model.** Throughput is a cliff, not a slope.
- **Eligible turns skew early.** Lossy turns cluster late in a session, so a clean cohort
  leans toward opening reads and away from the edits that follow several observations.

## Layout

| | |
|---|---|
| `cascade/` | the sidecar: dispatcher, explorer, gate, ledger, dashboard |
| `registry/` | device tiering and the licence-gated model catalogue |
| `packaging/` | builds the single self-contained binary |
| `QUICKSTART.md` | the install guide, written for whoever owns the AI bill |

## Build a binary

```bash
python -m pip install pyinstaller httpx
python observe/packaging/build.py
```

Carries its own Python. The machine it runs on needs nothing installed.
