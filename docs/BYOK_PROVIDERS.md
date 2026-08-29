# Bring Your Own Key (BYOK) Providers

Kerna relies on a flexible BYOK model to ensure that you are in control of your keys, models, and data routing. This is critical for both security and cost management.

## Defining Providers

Providers are configured via the CLI or directly in `kerna.toml`. 

```bash
kerna provider add my-openai \
    --provider-type openai \
    --api-key-env OPENAI_API_KEY \
    --default-model gpt-4o-mini
```

When a provider uses `--api-key-env`, Kerna reads the key from the environment variable at runtime. **Keys are never stored in plain text in the Kerna configuration file.**

## Model Routes

Model routes allow you to define semantic aliases for your models. Instead of hardcoding `gpt-4o-mini` across your tasks, you can route tasks to `cheap` or `smart`.

```bash
kerna provider route set cheap my-openai/gpt-4o-mini
kerna provider route set smart anthropic/claude-sonnet-4-20250514
```

Routes are selected by an explicit privacy label in `kerna.toml`; there is no
silent fallback to a different provider:
```bash
kerna provider route resolve project
kerna run "Summarize this file" --privacy project
```

## Privacy Routes

Privacy routes act as hard constraints. Bind a privacy label to a named model
route in `kerna.toml`; `local-only` additionally verifies the selected endpoint
is loopback before a task starts.

```toml
[model_routes]
offline-coding = "ollama/qwen2.5-coder:7b"
project-default = "openrouter/openai/gpt-4o-mini"

[privacy_routes]
local-only = "offline-coding"
project = "project-default"
```

Use `kerna provider route resolve local-only` to show the selected endpoint and
`kerna provider models ollama` to discover models actually installed locally.
Kerna does not maintain a hard-coded local-model catalogue: a local runtime is
the source of truth for which models are available.

## Listing Providers

You can audit your configured providers and routes at any time:
```bash
kerna provider list
```
