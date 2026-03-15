# srm-monitor

Rust monitor for Synology SRM mesh uplink state. The binary logs the selected uplink band and average RX/TX rates to CSV every second, prints a single console line on startup and whenever the selected band changes, opens a native hardware-accelerated telemetry window, and performs session cleanup on `SIGINT`, `SIGTERM`, or window close.

## Configuration

Create `srm-monitor/secrets/srm_login.toml` with your SRM credentials:

```toml
[credentials]
username = "your-username"
password = "your-password"
```

The `secrets/` directory is ignored by git.

## Runtime Behavior

- CSV output is written to `srm-monitor/avg_rates.csv`.
- A native window is created with the `wgpu` renderer, which maps to Metal on macOS hardware.
- The window reads `avg_rates.csv` on startup so previous samples are visible immediately.
- The window updates in real time as fresh SRM samples arrive and shows the current band, RX/TX throughput history, and signal strength history.
- The charts use local wall clock time on the x-axis, default to a rolling five minute view, and can be panned or zoomed on the x-axis to inspect the full retained history.
- The throughput chart uses a fixed 0 to 2000 Mbps y-axis and the signal chart uses a fixed 0 to 105 percent y-axis.
- The signal value reported by SRM is treated as a percentage, not dBm.
- CSV rows use ISO8601 timestamps with offset: `2026-03-14T21:14:38+00:00`.
- Console output uses local timezone abbreviations such as `GMT` or `BST`.
- The first selected band is printed once, and later output is only emitted when the selected band changes.
- `SIGINT`, `SIGTERM`, and closing the window trigger a clean shutdown and explicit Synology session cleanup.

Example console line:

```text
Sat 14th Mar 2026 21:55 GMT band=5G-1 signalstrength=-55 tx=1.404 Gbps rx=1.300 Gbps
```

Example CSV:

```csv
timestamp,band,signalstrength,avg_rx_bps,avg_tx_bps
2026-03-14T21:14:38+00:00,5G-1,-55,1300000000,1733000000
```

## Development

Build:

```bash
cargo build
```

Run:

```bash
cargo run
```

On macOS this opens the live graph window and uses the `wgpu` backend so rendering goes through the system GPU stack.

Lint:

```bash
cargo clippy -- -D warnings
```

Test:

```bash
cargo test
```

## Test Coverage

The current test suite covers:

- monitor control logic and retry flow with mocked session connectors
- band-change formatting and DST-aware timestamp rendering
- Synology response parsing and connected-uplink selection
- error handling for missing nodes, disconnected uplinks, and HTTP status validation