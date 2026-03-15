# srm-monitor

Workspace for three independent SRM telemetry applications backed by a shared Rust library.

## Applications

- `srm-monitor-service`: polls Synology SRM and writes telemetry samples into MongoDB.
- `srm-data-api`: serves telemetry samples from MongoDB over HTTP as JSON.
- `srm-graph-gui`: native `wgpu` GUI that queries the HTTP API and plots the returned data.
- `srm-common`: shared library for config loading, Synology API access, and telemetry models.

Each runnable application can compile and run independently.

## Layout

```text
srm-common/
srm-monitor-service/
srm-data-api/
srm-monitor/
```

## Configuration

Each runnable application reads a TOML config file from its own `config/` folder. Example files are committed, while the real `.toml` files are gitignored.

- `srm-monitor-service/config/service.example.toml`
- `srm-data-api/config/api.example.toml`
- `srm-monitor/config/gui.example.toml`

Default runtime config paths:

- `srm-monitor-service/config/service.toml`
- `srm-data-api/config/api.toml`
- `srm-monitor/config/gui.toml`

When launched from the workspace root with `cargo run -p ...`, each application resolves its default config relative to its own crate directory.

Optional environment variables can override those paths:

- `SRM_MONITOR_SERVICE_CONFIG`
- `SRM_DATA_API_CONFIG`
- `SRM_GRAPH_GUI_CONFIG`

For the GUI, `history_start` is the oldest timestamp the client will request from the API. On startup it loads the most recent five minutes, and after that it keeps only the currently displayed time range in memory. Pan or zoom to a different range and the GUI requests that range from the API instead of caching full history locally.

## Docker Compose

The repository includes [docker-compose.yml](docker-compose.yml) to start MongoDB, the monitor service, and the data API together.

Create a local `.env` from [.env.example](.env.example) and fill in the Synology credentials before starting the stack.

To start the backend stack and then launch the native GUI with one command, run:

```bash
./scripts/start-gui-stack.sh
```

The launcher will:

- read Synology credentials from `.env` or fall back to `srm-monitor/secrets/srm_login.toml`
- start Docker Compose for MongoDB, the monitor service, and the API
- wait for the API to answer on `http://127.0.0.1:8080`
- create `srm-monitor/config/gui.toml` if it does not already exist
- launch `cargo run -p srm-graph-gui`

By default, when the GUI exits, the launcher also stops the compose stack. Pass `--keep-backend` if you want the containers left running after the GUI closes.

Start the stack:

```bash
docker compose up --build -d
```

Stop the stack:

```bash
docker compose down
```

The API will be available at `http://127.0.0.1:8080/telemetry` on the host, while the services use the internal MongoDB hostname `mongodb` inside the compose network.

## Run

Start the Mongo writer:

```bash
cargo run -p srm-monitor-service
```

Start the HTTP API:

```bash
cargo run -p srm-data-api
```

Start the GUI:

```bash
cargo run -p srm-graph-gui
```

The GUI queries `/telemetry` with RFC3339 `start` and `end` parameters and renders the JSON response.

## Development

Format:

```bash
cargo fmt --all
```

Test:

```bash
cargo test
```

## Profiling

The GUI supports optional local profiling output:

```bash
cd srm-monitor
SRM_PROFILE=1 cargo run
```

When enabled, profiling output is written under `srm-monitor/instrumentation/latest/` as:

- `trace.ndjson`
- `summary.json`

That folder is gitignored.