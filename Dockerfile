# syntax=docker/dockerfile:1.7

FROM rust:1.97.0-bookworm AS builder
WORKDIR /build

COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY src ./src
COPY benches ./benches
COPY tool-packs ./tool-packs

RUN cargo build --locked --release --bins

COPY solution-packs ./solution-packs

FROM debian:bookworm-slim AS runtime

ARG AMOS_VERSION=0.2.0
LABEL org.opencontainers.image.title="AMOS" \
      org.opencontainers.image.description="Customer-evaluation server for governed analytical workflows" \
      org.opencontainers.image.version="${AMOS_VERSION}" \
      org.opencontainers.image.source="https://github.com/EltonChang1/AMOS"

RUN groupadd --gid 10001 amos \
    && useradd --uid 10001 --gid 10001 --no-create-home --shell /usr/sbin/nologin amos \
    && install -d -o amos -g amos /var/lib/amos

COPY --from=builder /build/target/release/amos /usr/local/bin/amos
COPY --from=builder /build/target/release/amosctl /usr/local/bin/amosctl
COPY --from=builder /build/solution-packs /usr/share/amos/solution-packs

USER 10001:10001
WORKDIR /var/lib/amos
EXPOSE 8000

ENTRYPOINT ["/usr/local/bin/amos"]
CMD ["--config", "/etc/amos/server.json", "serve"]
