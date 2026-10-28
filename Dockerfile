# --- build stage -----------------------------------------------------------
FROM rust:1.88-slim AS build
WORKDIR /app
# minimal deps: rustls => no openssl needed; bundled sqlite only when feature on
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY src ./src
# Cargo parses explicitly declared benchmark targets even for a bin build.
COPY benches ./benches
ARG HARVEST_FEATURES="sqlite postgres redis"
ARG CARGO_BUILD_JOBS=2
ENV CARGO_BUILD_JOBS=$CARGO_BUILD_JOBS
RUN cargo build --locked --release --features "$HARVEST_FEATURES" && \
    cp target/release/harvest /harvest

# --- runtime stage ----------------------------------------------------------
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/* && useradd -r -u 10001 harvest && \
    mkdir -p /var/lib/harvest /etc/harvest && chown -R harvest:harvest /var/lib/harvest
COPY --from=build /harvest /usr/local/bin/harvest
USER harvest
EXPOSE 9678
VOLUME ["/var/lib/harvest"]
# Liveness via harvest's built-in /health client. Exec form keeps the
# runtime image independent of bash/curl/wget and returns a failing exit code
# when the live admin endpoint is unhealthy. AdminClient takes HOST:PORT
# (not a scheme-bearing URL); the CLI probe targets :9678.
HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --retries=3 \
    CMD ["harvest", "health", "--url", "127.0.0.1:9678", "--json"]
ENTRYPOINT ["harvest"]
CMD ["run", "--config", "/etc/harvest/config.json"]
