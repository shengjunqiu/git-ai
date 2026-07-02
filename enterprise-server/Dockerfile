# ============================================================
# Stage 1: Build
# ============================================================
FROM docker.m.daocloud.io/library/rust:1.93-slim-bookworm AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    build-essential \
    git \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Copy full source
COPY . .

# Build release binary
RUN cargo build --release --locked --bin git-ai-enterprise-server

# ============================================================
# Stage 2: Runtime
# ============================================================
FROM docker.m.daocloud.io/library/debian:bookworm-slim AS runner

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -r -u 1001 -g root -s /sbin/nologin gitai

# Copy binary
COPY --from=builder /build/target/release/git-ai-enterprise-server /usr/local/bin/git-ai-enterprise-server

# Data directory
RUN mkdir -p /data && chown gitai /data

USER gitai
WORKDIR /data

# Default environment
ENV RUST_LOG=git_ai_enterprise_server=info,tower_http=warn
ENV LISTEN_ADDR=0.0.0.0:8080

EXPOSE 8080

# Health check
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

CMD ["/usr/local/bin/git-ai-enterprise-server"]
