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

Optional environment variables can override those paths:

- `SRM_MONITOR_SERVICE_CONFIG`
- `SRM_DATA_API_CONFIG`
- `SRM_GRAPH_GUI_CONFIG`

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