# Single-dashboard demo

From PowerShell in the verified local checkout, before sharing the screen:

```powershell
cd D:\Kerna-MVP\repos\kerna
kerna demo start
```

Enter Anthropic and Tenki keys at the two hidden prompts. They remain in the
trusted supervisor/adapter, never in the agent environment or generated files.
Wait for `Ready`. Tenki provisioning happens here (up to 70 seconds), not once
per stage command. The cloud VM has inbound/outbound networking disabled and a
15-minute lifetime. Start presenting promptly; restart preparation if it expires.
An existing dashboard on port 8877 must be closed first, or pass `--port 8890`.

At the **kerna-demo>** prompt, run these one at a time (not in PowerShell):

```text
cloud-allow
cloud-deny
local-allow
local-deny
```

- Cloud: Anthropic primary plus tool-less local shadow, execution via Tenki.
- Local: Ollama primary, execution via Wasmer; no cloud shadow.
- Allow: Kerna admits a bounded computation; successful output is `285`.
- Deny: Kerna rejects a request for environment variables before delegation.
  The sandbox is **not contacted** for that action. This is a Kerna policy proof,
  not a claimed in-guest enforcement demonstration.

All cases use one disposable SSD clone and one dashboard/evidence store, with
separate model-session credentials and sticky routes. The model must actually
issue the tool call: a model refusal or provider failure is not a policy proof.
The allow cases use automatic policy admission, not browser human approval.
The environment check is deliberately conservative demo admission, not a
complete Python security analyzer. Sponsor isolation remains necessary.

In the premium dashboard show Routing, Primary vs Shadow, Receipt report,
Containment, then click **View** on both allowed and blocked receipts to open
Audit detail. Export evidence before typing `exit`, which closes the dashboard
and requests remote VM cleanup. If cleanup cannot be confirmed, the CLI warns;
the remote lifetime remains the fallback. Do not abruptly terminate the process.

Runtime costs/latency depend on the providers. Four live model turns are not
guaranteed to finish within two minutes; rehearse with real keys before staging.
The synthetic history remains visibly labelled. The host Claude process and
trusted gateway are not fully containerized. Wasmer/Tenki isolate delegated code.

Validation: 202 Rust tests, JavaScript syntax check, and browser inspection of a
blocked receipt opening Audit. Full live cloud/shadow/Tenki rehearsal requires
the presenter's keys and is not implied by automated-test success.
