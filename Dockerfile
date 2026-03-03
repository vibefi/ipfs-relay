# syntax=docker/dockerfile:1

# ── Stage 1: cargo-chef planner ──────────────────────────────────────────────
# Computes a "recipe" (dependency manifest) so layer caching skips dep
# recompilation when only src/ changes.
FROM lukemathwalker/cargo-chef:latest-rust-1-bookworm AS chef
WORKDIR /build

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ── Stage 2: dependency builder ───────────────────────────────────────────────
FROM chef AS builder
COPY --from=planner /build/recipe.json recipe.json
# Build deps only — this layer is cached until Cargo.toml/Cargo.lock changes
RUN cargo chef cook --release --recipe-path recipe.json
# Build the actual binary
COPY . .
RUN cargo build --release --bin ipfs-relay

# ── Stage 3: minimal runtime image ───────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /build/target/release/ipfs-relay /app/ipfs-relay
COPY config/ /app/config/

# Non-root user
RUN useradd -r -u 1001 relay
RUN chown -R relay:relay /app
USER relay

EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

ENTRYPOINT ["/app/ipfs-relay"]
