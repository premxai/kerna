# Qoder governed-MCP demo

This is the smallest honest Kerna/Qoder demonstration: Qoder is an MCP client,
and Kerna governs only the MCP tools that Qoder calls through `kerna gateway`.
It does **not** claim to govern Qoder's native editor, terminal, or built-in
tools.

## Prepare a demo workspace

From the repository root (or from a Kerna binary installation):

```powershell
kerna contract init --template deployment-assistant --name "Qoder demo" --output .\demo-workspace
kerna client config --client qoder --workspace .\demo-workspace | Set-Content .\demo-workspace\.mcp.json
```

Open `demo-workspace` as the Qoder project. The generated `kerna.toml` starts
Kerna's built-in deterministic MockMCP child. It explicitly allows only `echo`
and explicitly denies `network_probe`; every other tool is denied.

## Connect Qoder

The generated configuration goes at `<project>/.mcp.json`. Qoder may require you to
trust/reload the project and enable Agent Mode before it starts local MCP
servers. Ensure the Kerna binary is on `PATH`; for a source checkout, use the
debug or release binary you built.

## Live sequence

1. Ask Qoder to list its Kerna MCP tools and call `echo` with a short greeting.
   Kerna forwards this approved call.
2. Ask Qoder to call `network_probe`.
   Kerna returns an MCP error before MockMCP can execute it.
3. Show the generated `agent-contract.md` and the `kerna.toml` permission rules.
4. Show the resulting `kerna.db` or run the verifier below to demonstrate the
   exact JSON-RPC boundary.

For a terminal-only rehearsal, run the verifier from inside `demo-workspace`:

```powershell
$env:KERNA_BIN = "C:\path\to\kerna.exe"
python ..\examples\qoder-governed-mcp\verify_gateway.py
```

It uses the same stdio MCP messages Qoder would use and checks both the approved
and denied paths. It is intentionally a local fixture; replace MockMCP only
after reviewing a real MCP server's tools and policy.
