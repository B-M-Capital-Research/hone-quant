# syntax=docker/dockerfile:1
#
# Self-contained image: the web UI is built first and embedded in the server binary.
#   docker build -t hone-quant --build-arg HONE_QUANT_REVISION=$(git rev-parse HEAD) .
# For local evaluation use docker-compose.yml (PostgreSQL + hone-quant in demo mode).
# Production on a VM uses the systemd release flow in docs/deployment-gce.md instead.

FROM oven/bun:1.3 AS web
WORKDIR /src/web
COPY web/package.json web/bun.lock ./
RUN bun install --frozen-lockfile
COPY web/ ./
RUN bun run build

FROM rust:1-bookworm AS server
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY config ./config
COPY --from=web /src/web/dist ./web/dist
ARG HONE_QUANT_REVISION=docker
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    HONE_QUANT_REVISION="$HONE_QUANT_REVISION" cargo build --release --locked -p quant-server \
    && install -m 0755 target/release/hone-quant /usr/local/bin/hone-quant

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --home-dir /var/lib/hone-quant --shell /usr/sbin/nologin hone-quant
COPY --from=server /usr/local/bin/hone-quant /usr/local/bin/hone-quant
USER hone-quant
WORKDIR /var/lib/hone-quant
ENV HONE_QUANT_BIND=0.0.0.0:8090 \
    HONE_QUANT_STATE_DIR=/var/lib/hone-quant \
    HONE_QUANT_LOG_FORMAT=json
VOLUME ["/var/lib/hone-quant"]
EXPOSE 8090
HEALTHCHECK --interval=30s --timeout=5s --start-period=60s \
    CMD curl -fsS http://127.0.0.1:8090/api/health || exit 1
ENTRYPOINT ["hone-quant"]
CMD ["serve"]
