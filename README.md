# srm-monitor

Rust monitor for Synology SRM mesh uplink state. The binary logs the selected uplink band and average RX/TX rates to CSV every 30 seconds, prints a single console line on startup and whenever the selected band changes, and performs session cleanup on `SIGINT` and `SIGTERM`.

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
- CSV rows use ISO8601 timestamps with offset: `2026-03-14T21:14:38+00:00`.
- Console output uses local timezone abbreviations such as `GMT` or `BST`.
- The first selected band is printed once, and later output is only emitted when the selected band changes.
- `SIGINT` and `SIGTERM` trigger a clean shutdown and explicit Synology session cleanup.

Example console line:

```text
Sat 14th Mar 2026 21:55 GMT band=5G-1 tx=1.404 Gbps rx=1.300 Gbps
```

Example CSV:

```csv
timestamp,band,avg_rx_bps,avg_tx_bps
2026-03-14T21:14:38+00:00,5G-1,1300000000,1733000000
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