# Kerna Guard Policy

Kerna Guard evaluates agent actions through one protocol-neutral contract. Claude Code actions
from Anthropic Messages and actions preserved for the OpenAI Responses adapter are normalized to
the same `ActionIntent` fields before policy evaluation. Raw action arguments are not retained by
the policy object; it keeps a SHA-256 digest plus a redacted resource summary.

## Policy file

Place `kerna.policy.toml` in the directory where `kerna serve` starts. The current format is
version 1:

```toml
version = 1
default = "ask"

[[rules]]
id = "allow-source-reads"
effect = "allow"
kind = "file_read"
path = "src/*"

[[rules]]
id = "ask-before-installs"
effect = "ask"
risk_tag = "dependency_install"
```

Rules are ordered and first-match. A rule can select by `agent`, `kind`, original `tool`, path
glob, `executable`, `domain`, or `risk_tag`. Multiple selectors on one rule must all match.
Effects are `allow`, `ask`, and `deny`. Invalid syntax, an unsupported version, duplicate rule
IDs, empty selectors, malformed known actions, and immutable safety violations fail closed.

The example at `kerna.policy.example.toml` expresses the built-in balanced policy. That policy is
intended for the contained worktree sessions introduced in WP2. Docker remains the authoritative
execution boundary; an explicit policy file takes precedence, while a project with legacy
`kerna.toml` permission rules is converted with a deny default for unlisted actions.

The balanced classifier asks before shell network clients, compound shell expressions, dependency
installation, destructive commands, or new network domains. It recognizes only a small set of
offline build, test, inspection, and repository-status commands as automatically releasable.

## Built-in safety floor

Policy cannot allow access to known secret paths, the Docker control plane, or parent-path host
escape attempts. An `allow` match is reduced to `ask` for destructive or external-side-effect
actions and multi-file patches. These checks supplement container isolation; they are not a
replacement for it.

Known malformed actions are denied before any permissive rule can release them. The balanced
policy also denies unknown tools and unsupported protocol action types; a future adapter must add
a reviewed canonical mapping before those tools can become actionable. Legacy conversion retains
its existing permissive-default behavior for unlisted tool names.

## Compatibility

Legacy permission conversion retains the prior exact-tool-over-wildcard behavior. The original
shared Kerna/LocalM fixture continues to test that behavior. A second fixture at
`benchmarks/policy/guard-policy-conformance.json` checks equivalent path, program, domain, risk,
unknown, malformed, and secret-path cases through both protocol adapters.
