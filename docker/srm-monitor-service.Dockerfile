FROM rust:1.88-bookworm AS builder
WORKDIR /workspace

COPY Cargo.toml Cargo.lock ./
COPY srm-common ./srm-common
COPY srm-data-api ./srm-data-api
COPY srm-monitor ./srm-monitor
COPY srm-monitor-service ./srm-monitor-service

RUN cargo build --release -p srm-monitor-service

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
ENV SRM_MONITOR_SERVICE_CONFIG=/app/config/service.toml
COPY --from=builder /workspace/target/release/srm-monitor-service /usr/local/bin/srm-monitor-service
COPY docker/entrypoint-monitor-service.sh /usr/local/bin/entrypoint-monitor-service.sh
RUN chmod +x /usr/local/bin/entrypoint-monitor-service.sh

ENTRYPOINT ["/usr/local/bin/entrypoint-monitor-service.sh"]
