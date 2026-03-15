FROM rust:1.88-bookworm AS builder
WORKDIR /workspace

COPY Cargo.toml Cargo.lock ./
COPY srm-common ./srm-common
COPY srm-data-api ./srm-data-api
COPY srm-monitor ./srm-monitor
COPY srm-monitor-service ./srm-monitor-service
COPY srm-web-ui ./srm-web-ui

RUN cargo build --release -p srm-web-ui

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
ENV SRM_WEB_UI_CONFIG=/app/config/web.toml
COPY --from=builder /workspace/target/release/srm-web-ui /usr/local/bin/srm-web-ui
COPY docker/entrypoint-web-ui.sh /usr/local/bin/entrypoint-web-ui.sh
RUN chmod +x /usr/local/bin/entrypoint-web-ui.sh

EXPOSE 6000
ENTRYPOINT ["/usr/local/bin/entrypoint-web-ui.sh"]
