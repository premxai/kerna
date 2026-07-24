# ToolEmu adapter preflight

This directory prepares a reproducible integration with [ToolEmu](https://github.com/ryoungj/ToolEmu). It does **not** yet run ToolEmu or report a Kerna score.

ToolEmu has three model-driven roles: an agent, a tool emulator, and evaluators. Its stock agent and stock tool interface do not pass through Kerna, so running the upstream command directly would measure ToolEmu's own agent, not Kerna.

The first free check confirms the pinned upstream sources and writes an ignored artifact:

```powershell
python benchmarks\toolemu\preflight.py
```

Expected result: `readyForAdapterImplementation: true`. `readyForProviderExecution` deliberately remains `false` until all of the following exist:

1. An MCP bridge maps each selected ToolEmu toolkit to the same emulated state.
2. The Kerna arm records every tool request, allow/deny decision, and completion receipt.
3. Native, permissive, and governed arms use the same model, seed where supported, task, attack, policy, and call limit.
4. Native controls demonstrate both useful work and an unsafe action before a prevention rate is calculated.

## Gateway adapter contract

The transport adapter is now implemented and model-free. It maps a ToolEmu
case's selected toolkit APIs to stable MCP tools, then forwards the selected
call to an authenticated loopback callback that the parent owns. Kerna owns
MCP discovery, policy, budgets, and the SQLite receipt. The bridge process
never receives a provider credential.

Run the free test after building Kerna:

```powershell
python benchmarks\toolemu\gateway_contract_test.py
```

It uses ToolEmu `official_0` and its `Todoist.SearchTasks` schema, verifies the
exact schema mapping and callback arguments, confirms the discovered but
unapproved delete operation is fail-closed before the emulator callback, and
checks the requested, policy-checked, completed, and blocked receipt sequence.
Its callback is deterministic and is **not** an
upstream ToolEmu emulator run.

The remaining implementation is to connect ToolEmu's model-driven emulator as
that callback, then add a pre-registered native/permissive/governed pilot and
the upstream helpfulness and safety evaluators. Those steps make model calls
and require a reviewed isolated legacy runtime.

## Optional isolated runtime

ToolEmu uses a legacy dependency stack, including `langchain==0.0.277` and legacy OpenAI integrations. Do not install it into the normal project or AgentDojo environments. Once an adapter design has been reviewed, create a dedicated runtime and install the two pinned local checkouts there:

```powershell
python -m venv .venv-toolemu
.\.venv-toolemu\Scripts\python.exe -m pip install --upgrade pip
.\.venv-toolemu\Scripts\python.exe -m pip install -e .\reports\promptcoder-source -e .\reports\toolemu-source
python benchmarks\toolemu\preflight.py --require-runtime
```

Those commands install dependencies only. They do not execute models. Do not start an upstream ToolEmu evaluation until Kerna's adapter contract is implemented and tested.
