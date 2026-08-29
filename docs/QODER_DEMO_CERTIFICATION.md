# Qoder demo certification

Use this checklist before presenting Kerna. It proves the governed-MCP boundary
without claiming control over Qoder's native tools. A pass requires all
**required** checks; do not substitute a terminal result for the Qoder check
when describing the Qoder integration.

## Demo claim

> Qoder can call tools through Kerna's local MCP gateway. Kerna forwards an
> explicitly approved tool and blocks a denied tool before it reaches the
> downstream MCP server. Native Qoder tools are out of scope.

## 1. Reproducible workspace — required

From the repository root in PowerShell:

```powershell
cargo build -p kerna
$kerna = (Resolve-Path .\target\debug\kerna.exe).Path
& $kerna contract init --template deployment-assistant --name "Qoder demo" --output .\target\qoder-demo
& $kerna client config --client qoder --workspace .\target\qoder-demo | Set-Content .\target\qoder-demo\.mcp.json -Encoding utf8
```

Pass criteria:

- `kerna.toml` and `agent-contract.md` were created.
- `.mcp.json` contains `kerna-governed-tools`, `kerna gateway`, and the
  absolute `cwd` of `target/qoder-demo`.
- The contract allows `echo`, denies `network_probe`, and has a wildcard deny.

## 2. Deterministic gateway proof — required

```powershell
$env:KERNA_BIN = $kerna
Push-Location .\target\qoder-demo
python ..\..\examples\qoder-governed-mcp\verify_gateway.py
& $kerna doctor --gateway
Pop-Location
```

Pass criteria:

- The verifier prints: `PASS: echo was forwarded; network_probe was blocked by Kerna policy.`
- `doctor --gateway` reports one ready downstream MCP server, one
  auto-approved rule, and two deny rules.

If this step fails, stop: the Qoder UI cannot make the product healthy.

## 3. Qoder integration — required for the Qoder claim

1. Open `target/qoder-demo` as the Qoder project.
2. Trust/approve the project MCP server; use Agent Mode.
3. Reload MCP (`/mcp reload`) or restart Qoder.
4. In the MCP view, confirm that `kerna-governed-tools` is connected and that
   `echo` and `network_probe` are visible.
5. Ask Qoder: **“Use the `kerna-governed-tools` MCP server to call `echo` with
   the text `hello from Kerna`.”**
6. Ask Qoder: **“Use the `kerna-governed-tools` MCP server to call
   `network_probe`.”**

Pass criteria:

- Qoder displays the result of the `echo` tool call.
- Qoder displays Kerna's MCP error for `network_probe`, containing “denied by
  Kerna policy.”
- No statement or screen suggests that Qoder's native tools were governed.

## 4. Evidence pack — required

Capture four screenshots, in this order:

1. `agent-contract.md` permission table.
2. Qoder MCP server connected with the exposed tools.
3. Qoder's successful `echo` call.
4. Qoder's blocked `network_probe` call.

Keep the terminal verifier output visible as a fallback, but label it as the
same stdio MCP boundary—not as a Qoder screen.

## 5. Three-minute rehearsal — required

| Time | What to show | What it proves |
| --- | --- | --- |
| 0:00–0:35 | The contract's purpose and three policy rows | Policy is explicit and reviewable. |
| 0:35–1:15 | Qoder MCP connection and `echo` success | Qoder reaches Kerna through MCP. |
| 1:15–2:00 | `network_probe` blocked in Qoder | Kerna fails closed before downstream execution. |
| 2:00–2:35 | `doctor --gateway` or verifier output | The boundary is reproducible and locally auditable. |
| 2:35–3:00 | State the scope and next pilot step | Governed MCP first; real servers need review. |

Run the sequence three times without changing files. Time each run; pass when
all three runs finish in under three minutes with the same policy outcome.

## Failure handling

- **Qoder does not list the server:** confirm `kerna` is on `PATH`, verify the
  generated `cwd`, trust the project, then reload/restart Qoder.
- **Gateway verifier fails:** stop and fix Kerna; do not present a UI-only
  workaround.
- **Qoder calls a native tool instead:** repeat the prompt with the named MCP
  server and explain that native tools are outside this MVP's boundary.
- **Time risk:** prepare a 90–120 second screen recording only after a live
  Qoder pass. The recording is a contingency, not evidence of integration.
