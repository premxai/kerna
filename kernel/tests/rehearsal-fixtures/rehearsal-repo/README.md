# Rehearsal fixture

Input to `scripts/accept-contained-run.ps1`. The harness copies this directory
into a temporary throwaway repository, commits it, and runs a contained,
receipt-governed Claude Code session against the copy — never this directory.

`src/add.rs` returns `0` on purpose. The proof is that a governed edit turns it
into `a + b`, that the edit is reviewed and applied into a *separate* clean
target clone, and that the run is captured in a signed evidence bundle.

Nothing here is a build output, a live workspace, or a Cargo test target.
