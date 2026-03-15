FROM rust:1.88-bookworm AS builder
WORKDIR /workspace

COPY Cargo.toml Cargo.lock ./
COPY srm-common ./srm-common
COPY srm-data-api ./srm-data-api
COPY srm-monitor ./srm-monitor
COPY srm-monitor-service ./srm-monitor-service

RUN cargo build --release -p srm-data-api

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
ENV SRM_DATA_API_CONFIG=/app/config/api.toml
COPY --from=builder /workspace/target/release/srm-data-api /usr/local/bin/srm-data-api
COPY docker/entrypoint-data-api.sh /usr/local/bin/entrypoint-data-api.sh
RUN chmod +x /usr/local/bin/entrypoint-data-api.sh

EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/entrypoint-data-api.sh"]
