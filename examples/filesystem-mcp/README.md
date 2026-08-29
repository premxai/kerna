# Kerna real-filesystem MCP fixture

This fixture is a dependency-free MCP server used to prove Kerna's production
container boundary against real files. It runs in the pinned image
`python@sha256:d09d15e60962ca365d1cd544a48773bac9d33f2fb1b00f2aa0deec78ade7dc31`.
The server itself (`plugin/`) is mounted read-only from the project, while
fixture input (`read/`) and output (`write/`) have separate mount scopes.

It intentionally exposes `read_file`, `write_file`, and `network_probe` so
acceptance tests can verify read-only input, approval-gated writes, and network
isolation. It is a test fixture, not a generally trusted third-party plugin.

## Running it through Kerna

The committed `kerna.toml` and signed manifest are ready to use with Docker.
From this directory, run `kerna doctor --gateway` and then point a client at
`kerna gateway --workspace <this-directory>`. To rotate the fixture signature,
set an Ed25519 signing seed in `KERNA_FIXTURE_SIGNING_KEY` (a base64-encoded
32-byte value), then sign the manifest:

```powershell
kerna mcp sign-manifest --manifest manifest.toml --signing-key-env KERNA_FIXTURE_SIGNING_KEY
```

Copy the printed `manifest_sha256` and `signing_public_key` into `kerna.toml`
before running `kerna doctor --gateway` again. The fixture already sets
`read_file` to `auto_approve`, `write_file` to `require_confirmation`, and
`network_probe` to `auto_approve` to prove the container's network isolation.

For a client-shaped black-box check (initialize, tools/list, real read,
approval retry, and network isolation), run `python verify_gateway.py` after
building Kerna or set `KERNA_BIN` to the built executable.
