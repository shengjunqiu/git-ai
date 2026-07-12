# ============================================================
# Stage 1: Build
# ============================================================
FROM rust:1.93-slim-bookworm AS builder

# Install build dependencies (OpenSSL vendored via feature, but needs pkg-config & cc)
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    build-essential \
    git \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Copy manifests and the local path dependency before the application source.
COPY Cargo.toml Cargo.lock ./
COPY crates/git-ai-protocol ./crates/git-ai-protocol

# Copy full source
COPY . .

# Persist downloaded crates and compiled dependencies across source changes.
RUN --mount=type=cache,id=git-ai-report-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=git-ai-report-cargo-git,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,id=git-ai-report-cargo-target,target=/build/target,sharing=locked \
    mkdir -p /out \
    && cargo build --release --locked --bin git-ai \
    && cp target/release/git-ai /out/git-ai

# ============================================================
# Stage 2: Runtime
# ============================================================
FROM debian:bookworm-slim AS runner

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    git \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -r -u 1001 -g root -s /sbin/nologin gitai

# Copy binary
COPY --from=builder /out/git-ai /usr/local/bin/git-ai

# Data directory for SQLite database
RUN mkdir -p /data && chown gitai /data

USER gitai
WORKDIR /data

# Default environment
ENV GIT_AI_REPORT_ADDR=0.0.0.0:8787
ENV GIT_AI_REPORT_DB=/data/report.sqlite
ENV GIT_AI_ASYNC_MODE=false

EXPOSE 8787

# Entrypoint: allow overriding addr/db via env or CMD args
ENTRYPOINT ["/usr/local/bin/git-ai", "report", "server"]
CMD ["--addr", "0.0.0.0:8787", "--db", "/data/report.sqlite"]
