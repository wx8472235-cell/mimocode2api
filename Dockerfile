# syntax=docker/dockerfile:1

FROM rust:1-bookworm AS builder
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release && cp target/release/mimocode2api /app/mimocode2api

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

RUN useradd -r -u 10001 appuser \
    && mkdir -p /data \
    && chown appuser:appuser /data
USER appuser

COPY --from=builder /app/mimocode2api /usr/local/bin/mimocode2api

ENV MIMOCODE_PORT=8080 \
    MIMOCODE_CLIENT_FILE=/data/client \
    MIMOCODE_MODEL=mimo-auto \
    RUST_LOG=info,mimocode2api=debug

EXPOSE 8080
VOLUME ["/data"]

HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -fsS "http://localhost:${MIMOCODE_PORT}/health" || exit 1

ENTRYPOINT ["mimocode2api"]
