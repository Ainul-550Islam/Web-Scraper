# --- build stage -----------------------------------------------------------
FROM rust:1.88-slim AS build
WORKDIR /app
# minimal deps: rustls => no openssl needed; bundled sqlite only when feature on
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock* ./
COPY src ./src
RUN cargo build --release --features "sqlite postgres" && \
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
# Liveness via the auth-exempt /health probe; pure-bash /dev/tcp keeps the
# runtime image free of curl/wget. Assumes metrics_addr on :9678 (the
# default) — change here together with the config if you move the port.
HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --retries=3 \
    CMD bash -c 'exec 3<>/dev/tcp/127.0.0.1/9678 && printf "GET /health HTTP/1.0\r\n\r\n" >&3 && grep -q "\"ok\"" <&3'
ENTRYPOINT ["harvest"]
CMD ["run", "--config", "/etc/harvest/config.json"]
