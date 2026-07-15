# Kerna controlled-cohort release runbook

Use this runbook for the first 5-10 user cohort. It produces a versioned
GitHub release with CLI binaries, native desktop installers, and checksums. It
does not authorize a broad public launch: code signing and macOS notarization
are separate requirements for that later stage.

## Preconditions

- The current [cohort launch checklist](COHORT_LAUNCH_CHECKLIST.md) has no
  unresolved local engineering gate.
- All user-facing versions match the intended tag: `kernel/Cargo.toml`,
  `ui/src-tauri/Cargo.toml`, `ui/src-tauri/tauri.conf.json`, and
  `npm/package.json`.
- The worktree is reviewed and clean. Never tag an unreviewed local build.
- The release owner has a known cohort list and will use a disposable Google
  account/calendar for connector acceptance.

## Local preflight

From the repository root, run:

```powershell
cargo fmt --manifest-path kernel/Cargo.toml -- --check
cargo clippy --manifest-path kernel/Cargo.toml -- -D warnings
cargo test --manifest-path kernel/Cargo.toml
cargo audit

npm --prefix ui ci
npm --prefix ui run build
cargo fmt --manifest-path ui/src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path ui/src-tauri/Cargo.toml -- -D warnings
cargo test --manifest-path ui/src-tauri/Cargo.toml
```

For a Windows desktop-artifact check, run `npm --prefix ui run tauri build`.
This is a local proof only; distribute the tagged release artifact, never a
file copied from `target/`.

## Publish the cohort release

1. Commit the reviewed release changes and wait for CI to pass.
2. Create an annotated tag whose version exactly matches all version files,
   for example `v0.2.0`.
3. Push the tag. The release workflow rejects mismatched versions, builds the
   CLI and desktop installers for its supported platforms, and attaches a
   SHA-256 checksum file beside each artifact group.
4. Open the GitHub release and confirm every expected asset is present before
   sharing its URL:
   - CLI binary plus `<asset>.sha256` for Linux, macOS, and Windows;
   - `kerna-plugins.zip` plus `kerna-plugins.zip.sha256`; extract the verified
     archive beside a manually installed CLI so curated packs are available;
   - native desktop installer(s) plus a platform-specific
     `kerna-desktop-*-SHA256SUMS` file.

## Verify before sharing

Download from the GitHub release, then compare the artifact with its published
checksum. On PowerShell:

```powershell
Get-FileHash .\kerna-windows-x86_64.exe -Algorithm SHA256
Get-FileHash .\Kerna_0.2.0_x64-setup.exe -Algorithm SHA256
Get-FileHash .\kerna-plugins.zip -Algorithm SHA256
Get-Content .\kerna-desktop-windows-SHA256SUMS
```

On macOS or Linux, use `sha256sum -c <asset>.sha256` for a CLI asset, or run
`grep '<installer filename>' kerna-desktop-*-SHA256SUMS | sha256sum -c -` for
a desktop installer.

Give cohort members the release URL, release tag, exact expected checksum, and
the [Private Alpha Guide](PRIVATE_ALPHA.md). They must install the CLI before
the desktop app and set `KERNA_HOME` to the initialized workspace. Never ask a
member to send an OAuth refresh token, raw database, or calendar contents.

## Stop and rollback

Do not expand the cohort if a checksum differs, an artifact is missing, a live
connector acceptance check fails, a credential appears in a trace, or an
unreviewed action executes. Remove the release link from distribution, pause
the cohort, preserve only safe task IDs and sanitized traces, and fix the
underlying issue before cutting a new versioned tag.
