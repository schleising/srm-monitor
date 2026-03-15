#!/bin/sh
set -eu

escape_toml_string() {
    printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'
}

mkdir -p /app/config

bind_address=$(escape_toml_string "${SRM_API_BIND_ADDRESS:-0.0.0.0:8080}")
mongodb_url=$(escape_toml_string "${SRM_MONGODB_URL:-mongodb://mongodb:27017}")
mongodb_database=$(escape_toml_string "${SRM_MONGODB_DATABASE:-srm}")
mongodb_collection=$(escape_toml_string "${SRM_MONGODB_COLLECTION:-telemetry}")

cat > "$SRM_DATA_API_CONFIG" <<EOF
[server]
bind_address = "$bind_address"

[mongodb]
url = "$mongodb_url"
database = "$mongodb_database"
collection = "$mongodb_collection"
EOF

exec /usr/local/bin/srm-data-api
