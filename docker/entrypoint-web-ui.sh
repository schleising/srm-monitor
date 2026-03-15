#!/bin/sh
set -eu

escape_toml_string() {
    printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'
}

mkdir -p /app/config

bind_address=$(escape_toml_string "${SRM_WEB_BIND_ADDRESS:-0.0.0.0:6080}")
api_base_url=$(escape_toml_string "${SRM_WEB_API_BASE_URL:-http://data-api:6081}")
refresh_interval_secs=${SRM_WEB_REFRESH_INTERVAL_SECS:-30}
history_window_secs=${SRM_WEB_HISTORY_WINDOW_SECS:-43200}

cat > "$SRM_WEB_UI_CONFIG" <<EOF
[server]
bind_address = "$bind_address"

[api]
base_url = "$api_base_url"
refresh_interval_secs = $refresh_interval_secs
history_window_secs = $history_window_secs
EOF

exec /usr/local/bin/srm-web-ui
