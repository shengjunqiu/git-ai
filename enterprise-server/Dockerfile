# ============================================================
# Stage 1: Build
# ============================================================
FROM rust:1.93-slim-bookworm AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    build-essential \
    git \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build/enterprise-server

# Copy only files needed for compilation. Keeping static assets out of this
# layer prevents dashboard HTML edits from invalidating the Rust build cache.
COPY enterprise-server/Cargo.toml enterprise-server/Cargo.lock ./
COPY crates/git-ai-protocol /build/crates/git-ai-protocol
COPY install.sh install.ps1 /build/
COPY enterprise-server/src ./src
COPY enterprise-server/migrations ./migrations

# Persist downloaded crates and compiled dependencies across source changes.
# The target directory is a cache mount, so copy the final binary to /out
# before the mount is detached from this layer.
RUN --mount=type=cache,id=git-ai-enterprise-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=git-ai-enterprise-cargo-git,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,id=git-ai-enterprise-cargo-target,target=/build/enterprise-server/target,sharing=locked \
    mkdir -p /out \
    && cargo build --release --locked --bin git-ai-enterprise-server \
    && cp target/release/git-ai-enterprise-server /out/git-ai-enterprise-server

# ============================================================
# Stage 2: Runtime
# ============================================================
FROM debian:bookworm-slim AS runner

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -r -u 1001 -g root -s /sbin/nologin gitai

# Copy binary
COPY --from=builder /out/git-ai-enterprise-server /usr/local/bin/git-ai-enterprise-server

# Data directory
RUN mkdir -p /data && chown gitai /data

# Dashboard static assets. docker-compose mounts ./static over this path in local
# development so UI edits can be refreshed without rebuilding the API image.
COPY --chown=1001:0 enterprise-server/static /data/static

USER gitai
WORKDIR /data

# Default environment
ENV RUST_LOG=git_ai_enterprise_server=info,tower_http=warn
ENV LOG_FORMAT=compact
ENV LISTEN_ADDR=0.0.0.0:8080

EXPOSE 8080

# Health check
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

CMD ["/usr/local/bin/git-ai-enterprise-server"]
