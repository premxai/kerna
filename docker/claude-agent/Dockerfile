# syntax=docker/dockerfile:1.7
FROM rust:1.88-bookworm@sha256:af306cfa71d987911a781c37b59d7d67d934f49684058f96cf72079c3626bfe0 AS builder
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY kernel/Cargo.toml ./kernel/
COPY kernel/src ./kernel/src
COPY kernel/assets ./kernel/assets
COPY kernel/registry ./kernel/registry
COPY kernel/tests ./kernel/tests
RUN cargo build --locked --release --package kerna

FROM node:22-bookworm@sha256:8a34c4ab3ea2c5cd194f07e317b2a8f09461d3c8b05c4e34c8ccd56d56024c4d
ARG CLAUDE_CODE_VERSION=2.1.270
ARG CLAUDE_CODE_INTEGRITY=sha512-0zMkfIWQu7/SG56VP8r780HZWvrNShzK28AbAnhKRK0ns+ToGXPT0W8UqyZmZCUKAkJDd5//TrwSOhk1+hysiw==
RUN test "$(npm view "@anthropic-ai/claude-code@${CLAUDE_CODE_VERSION}" dist.integrity)" = "${CLAUDE_CODE_INTEGRITY}" \
    && npm install --global "@anthropic-ai/claude-code@${CLAUDE_CODE_VERSION}" \
    && npm cache clean --force \
    && useradd --create-home --uid 10001 --shell /bin/bash kerna
COPY --from=builder /src/target/release/kerna /usr/local/bin/kerna
LABEL dev.kerna.contract="claude-agent-v1" \
      dev.kerna.claude-code-version="2.1.270" \
      dev.kerna.claude-package="@anthropic-ai/claude-code@2.1.270" \
      dev.kerna.claude-package-integrity="sha512-0zMkfIWQu7/SG56VP8r780HZWvrNShzK28AbAnhKRK0ns+ToGXPT0W8UqyZmZCUKAkJDd5//TrwSOhk1+hysiw==" \
      dev.kerna.rust-base-digest="sha256:af306cfa71d987911a781c37b59d7d67d934f49684058f96cf72079c3626bfe0" \
      dev.kerna.node-base-digest="sha256:8a34c4ab3ea2c5cd194f07e317b2a8f09461d3c8b05c4e34c8ccd56d56024c4d" \
      dev.kerna.network="broker-only"
USER 10001:10001
WORKDIR /workspace
ENTRYPOINT []
CMD ["claude", "--version"]
