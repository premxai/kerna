# @premxai/kerna

The npm distribution of [Kerna](https://github.com/premxai/kerna) — the runtime trust layer for AI agents.

```bash
npm install -g @premxai/kerna
kerna init
```

Or run without installing:

```bash
npx @premxai/kerna init
```

On install, this package downloads the prebuilt `kerna` binary for your platform
(macOS arm64/x86_64, Linux x86_64, Windows x86_64) from GitHub Releases and
exposes it as the `kerna` command. For everything else — commands, providers,
MCP tools, the policy gateway — see the [main README](https://github.com/premxai/kerna#readme).

Prefer a native build? `cargo install --git https://github.com/premxai/kerna --bin kerna`.
