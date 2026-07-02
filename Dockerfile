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

# Copy dependency manifests first for better layer caching
COPY Cargo.toml Cargo.lock ./

# Pre-fetch dependencies by building a dummy main (cache layer)
RUN mkdir -p src && echo 'fn main(){}' > src/main.rs && \
    cargo fetch --locked && \
    rm -rf src

# Copy full source
COPY . .

# Build release binary (report server only needs the main binary)
RUN cargo build --release --locked --bin git-ai

# ============================================================
# Stage 2: Runtime
# ============================================================
FROM debian:bookworm-slim AS runner

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    git \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -r -u 1001 -g root -s /sbin/nologin gitai

# Copy binary
COPY --from=builder /build/target/release/git-ai /usr/local/bin/git-ai

# Data directory for SQLite database
RUN mkdir -p /data && chown gitai /data

USER gitai
WORKDIR /data

# Default environment
ENV GIT_AI_REPORT_ADDR=0.0.0.0:8787
ENV GIT_AI_REPORT_DB=/data/report.sqlite

EXPOSE 8787

# Entrypoint: allow overriding addr/db via env or CMD args
ENTRYPOINT ["/usr/local/bin/git-ai", "report", "server"]
CMD ["--addr", "0.0.0.0:8787", "--db", "/data/report.sqlite"]
