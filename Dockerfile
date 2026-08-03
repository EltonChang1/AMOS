FROM rust:1.97-bookworm AS builder

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY demo ./demo
COPY benches ./benches
RUN cargo build --release --locked

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 amos \
    && useradd --uid 10001 --gid amos --home-dir /nonexistent --shell /usr/sbin/nologin amos \
    && install --directory --owner=amos --group=amos /data

COPY --from=builder /build/target/release/amos /usr/local/bin/amos

USER 10001:10001
VOLUME ["/data"]
EXPOSE 8000
HEALTHCHECK --interval=10s --timeout=3s --start-period=10s --retries=6 \
    CMD ["curl", "--fail", "--silent", "http://127.0.0.1:8000/health"]

ENTRYPOINT ["/usr/local/bin/amos", "--demo", "--root", "/data"]
CMD ["serve", "--seed-demo", "--bind", "0.0.0.0", "--port", "8000"]
