#!/bin/sh
set -eu

escape_toml_string() {
    printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'
}

: "${SRM_SYNOLOGY_USERNAME:?SRM_SYNOLOGY_USERNAME is required}"
: "${SRM_SYNOLOGY_PASSWORD:?SRM_SYNOLOGY_PASSWORD is required}"

mkdir -p /app/config

synology_base_url=$(escape_toml_string "${SRM_SYNOLOGY_BASE_URL:-http://192.168.1.1:8000/webapi}")
synology_username=$(escape_toml_string "$SRM_SYNOLOGY_USERNAME")
synology_password=$(escape_toml_string "$SRM_SYNOLOGY_PASSWORD")
mongodb_url=$(escape_toml_string "${SRM_MONGODB_URL:-mongodb://mongodb:27017}")
mongodb_database=$(escape_toml_string "${SRM_MONGODB_DATABASE:-srm}")
mongodb_collection=$(escape_toml_string "${SRM_MONGODB_COLLECTION:-telemetry}")

cat > "$SRM_MONITOR_SERVICE_CONFIG" <<EOF
[synology]
base_url = "$synology_base_url"
node_id = ${SRM_SYNOLOGY_NODE_ID:-8}
poll_interval_secs = ${SRM_SYNOLOGY_POLL_INTERVAL_SECS:-30}

[synology.credentials]
username = "$synology_username"
password = "$synology_password"

[mongodb]
url = "$mongodb_url"
database = "$mongodb_database"
collection = "$mongodb_collection"
EOF

exec /usr/local/bin/srm-monitor-service
