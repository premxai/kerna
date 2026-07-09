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
kerna route add cheap my-openai/gpt-4o-mini
kerna route add smart anthropic/claude-sonnet-4-20250514
```

Now you can instruct Kerna to run a task using a route:
```bash
kerna run "Summarize this file" --route cheap
```

## Privacy Routes

Privacy routes act as hard constraints. If a task requires absolute data sovereignty, you can bind the `local_only` privacy route to a local provider like Ollama.

```bash
kerna privacy-route add local_only ollama/llama3
```

If a task is executed under the `local_only` context, Kerna will mathematically guarantee that the data never leaves the machine.

## Listing Providers

You can audit your configured providers and routes at any time:
```bash
kerna provider list
```
