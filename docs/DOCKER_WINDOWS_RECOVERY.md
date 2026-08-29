# Docker Desktop Windows stale-socket recovery

If Docker Desktop shows an error such as:

```text
initializing Ingest server: ... sailor-ingest.sock:
remove ...: The file cannot be accessed by the system.
```

this is a Docker Desktop AF_UNIX socket reparse-point left by an unclean
shutdown, not a Kerna configuration error. Docker's current Windows issue
tracker documents the same failure and recommends renaming the socket
directories so Desktop can recreate them:
[docker/desktop-feedback#554](https://github.com/docker/desktop-feedback/issues/554).

1. Quit Docker Desktop completely.
2. In PowerShell, verify the destination names do not already exist, then run:

```powershell
docker desktop stop
Move-Item "$env:LOCALAPPDATA\Docker\run" "$env:LOCALAPPDATA\Docker\run.stale"
New-Item -ItemType Directory "$env:LOCALAPPDATA\Docker\run"
Move-Item "$env:LOCALAPPDATA\docker-secrets-engine" "$env:LOCALAPPDATA\docker-secrets-engine.stale"
New-Item -ItemType Directory "$env:LOCALAPPDATA\docker-secrets-engine"
docker desktop start
docker version
```

The `.stale` directories are preserved for rollback; do not delete them until
Docker has started and the images/volumes you need are visible. If a `.stale`
destination already exists, choose a new suffix (for example
`run.stale.2`) instead of overwriting it. Kerna will then report Docker as
available through `kerna doctor --gateway`.
