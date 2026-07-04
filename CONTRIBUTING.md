# Contributing to Kerna

First off, thank you for considering contributing to Kerna! It's people like you that make Kerna the most robust, observable agent runtime.

## Development Setup

1. Ensure you have Rust and Cargo installed.
2. Clone the repository.
3. Run `cargo build` and `cargo test` in the `kernel/` directory.

## Submitting Pull Requests

1. **Fork the repo** and create your branch from `main`.
2. **Write tests** for your changes. We enforce strict test coverage for `permissions.rs` and `memory.rs`.
3. Ensure your code passes all linting (`cargo clippy`) and formatting (`cargo fmt`).
4. Keep the PR focused on a single change.

## Feature Requests

We are very selective about new features. Kerna is designed to be a lightweight OS, not a bloated framework. If your feature can be implemented as an MCP plugin, we will likely ask you to build it as a plugin rather than adding it to the Core Runtime.
