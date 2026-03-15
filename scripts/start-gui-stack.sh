#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname "$0")" && pwd)
REPO_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
ENV_FILE="$REPO_ROOT/.env"
GUI_CONFIG_PATH="$REPO_ROOT/srm-monitor/config/gui.toml"
COMPOSE_FILE="$REPO_ROOT/docker-compose.yml"
GUI_API_PORT=${SRM_GUI_API_PORT:-6081}
COMPOSE_OVERRIDE_FILE=

API_BASE_URL=${SRM_GUI_API_BASE_URL:-http://127.0.0.1:$GUI_API_PORT}
GUI_REFRESH_INTERVAL_SECS=${SRM_GUI_REFRESH_INTERVAL_SECS:-1}
GUI_HISTORY_START=${SRM_GUI_HISTORY_START:-1970-01-01T00:00:00Z}
API_HEALTHCHECK_END=${SRM_API_HEALTHCHECK_END:-2100-01-01T00:00:00Z}
GUI_COMMAND=${SRM_GUI_COMMAND:-cargo run -p srm-graph-gui}
KEEP_BACKEND=0
BACKEND_ONLY=0

usage() {
    cat <<'EOF'
Usage: ./scripts/start-gui-stack.sh [--keep-backend] [--backend-only]

Options:
  --keep-backend  Leave the Docker Compose stack running after the GUI exits.
  --backend-only  Start the backend and wait for the API, but do not launch the GUI.
EOF
}

for arg in "$@"; do
    case "$arg" in
        --keep-backend)
            KEEP_BACKEND=1
            ;;
        --backend-only)
            BACKEND_ONLY=1
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "unknown argument: $arg" >&2
            usage >&2
            exit 1
            ;;
    esac
done

load_env_file() {
    if [ -f "$ENV_FILE" ]; then
        set -a
        # shellcheck disable=SC1090
        . "$ENV_FILE"
        set +a
    fi
}

ensure_compose_override() {
        COMPOSE_OVERRIDE_FILE=$(mktemp "${TMPDIR:-/tmp}/srm-gui-compose.XXXXXX.yml")
        cat > "$COMPOSE_OVERRIDE_FILE" <<EOF
services:
    data-api:
        ports:
            - "$GUI_API_PORT:6081"
EOF
}

docker_compose() {
        docker compose -f "$COMPOSE_FILE" -f "$COMPOSE_OVERRIDE_FILE" "$@"
}

require_credentials() {
    if [ -z "${SRM_SYNOLOGY_USERNAME:-}" ] || [ -z "${SRM_SYNOLOGY_PASSWORD:-}" ]; then
        echo "missing Synology credentials; set them in .env" >&2
        exit 1
    fi
}

ensure_gui_config() {
    current_refresh_interval_secs=$GUI_REFRESH_INTERVAL_SECS
    current_history_start=$GUI_HISTORY_START

    if [ -f "$GUI_CONFIG_PATH" ]; then
        existing_refresh_interval_secs=$(awk -F'= ' '/^refresh_interval_secs/ { gsub(/[[:space:]]/, "", $2); print $2; exit }' "$GUI_CONFIG_PATH")
        existing_history_start=$(awk -F'"' '/^history_start/ { print $2; exit }' "$GUI_CONFIG_PATH")

        if [ -n "$existing_refresh_interval_secs" ]; then
            current_refresh_interval_secs=$existing_refresh_interval_secs
        fi

        if [ -n "$existing_history_start" ]; then
            current_history_start=$existing_history_start
        fi
    fi

    mkdir -p "$(dirname "$GUI_CONFIG_PATH")"
    cat > "$GUI_CONFIG_PATH" <<EOF
[api]
base_url = "$API_BASE_URL"
refresh_interval_secs = $current_refresh_interval_secs
history_start = "$current_history_start"
EOF
}

wait_for_api() {
    echo "waiting for API at $API_BASE_URL"
    attempt=0
    until curl -fsS --get \
        --data-urlencode "start=$GUI_HISTORY_START" \
        --data-urlencode "end=$API_HEALTHCHECK_END" \
        "$API_BASE_URL/telemetry" >/dev/null; do
        attempt=$((attempt + 1))
        if [ "$attempt" -ge 60 ]; then
            echo "API did not become ready in time" >&2
            return 1
        fi
        sleep 1
    done
}

cleanup() {
    if [ "$KEEP_BACKEND" -eq 1 ]; then
        if [ -n "$COMPOSE_OVERRIDE_FILE" ] && [ -f "$COMPOSE_OVERRIDE_FILE" ]; then
            rm -f "$COMPOSE_OVERRIDE_FILE"
        fi
        return
    fi

    echo "stopping compose stack"
    cd "$REPO_ROOT"
    docker_compose down

    if [ -n "$COMPOSE_OVERRIDE_FILE" ] && [ -f "$COMPOSE_OVERRIDE_FILE" ]; then
        rm -f "$COMPOSE_OVERRIDE_FILE"
    fi
}

launch_gui() {
    echo "launching GUI"
    sh -c "$GUI_COMMAND"
}

load_env_file
require_credentials
ensure_compose_override

trap cleanup EXIT INT TERM

ensure_gui_config

cd "$REPO_ROOT"
echo "starting compose stack"
docker_compose up --build -d
wait_for_api

if [ "$BACKEND_ONLY" -eq 1 ]; then
    echo "backend ready; GUI launch skipped"
    exit 0
fi

launch_gui