use anyhow::Result;
use axum::extract::{Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::{Router, routing::get};
use reqwest::Client;
use serde::Deserialize;
use srm_common::config::{WebConfig, env_or_manifest_path, load_toml_file};

const APP_NAME: &str = env!("CARGO_PKG_NAME");
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const CONFIG_ENV_VAR: &str = "SRM_WEB_UI_CONFIG";
const DEFAULT_CONFIG_PATH: &str = "config/web.toml";

#[derive(Clone)]
struct AppState {
    http_client: Client,
    api_base_url: String,
    refresh_interval_secs: u64,
    history_window_secs: u64,
}

#[derive(Deserialize)]
struct TelemetryQuery {
    start: String,
    end: String,
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("error=fatal details={}", error);
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    println!("{} v{}", APP_NAME, APP_VERSION);
    let config_path = env_or_manifest_path(
        CONFIG_ENV_VAR,
        DEFAULT_CONFIG_PATH,
        env!("CARGO_MANIFEST_DIR"),
    );
    let config: WebConfig = load_toml_file(&config_path)?;

    let bind_address = config.server.bind_address.clone();
    let state = build_state(config);
    let app = build_app(state);

    let listener = tokio::net::TcpListener::bind(&bind_address).await?;
    println!("listening=http://{}", bind_address);
    axum::serve(listener, app).await?;
    Ok(())
}

fn build_state(config: WebConfig) -> AppState {
    AppState {
        http_client: Client::new(),
        api_base_url: config.api.base_url.trim_end_matches('/').to_string(),
        refresh_interval_secs: config.api.refresh_interval_secs.max(1),
        history_window_secs: config.api.history_window_secs.max(60),
    }
}

fn build_app(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/favicon.svg", get(favicon))
        .route("/app.js", get(app_js))
        .route("/styles.css", get(styles_css))
        .route("/api/telemetry", get(proxy_telemetry))
        .with_state(state)
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn favicon() -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("image/svg+xml; charset=utf-8"),
        )],
        FAVICON_SVG,
    )
}

async fn styles_css() -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/css; charset=utf-8"),
        )],
        STYLES_CSS,
    )
}

async fn app_js(State(state): State<AppState>) -> impl IntoResponse {
    let script = render_app_js(state.refresh_interval_secs, state.history_window_secs);
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/javascript; charset=utf-8"),
        )],
        script,
    )
}

async fn proxy_telemetry(
    State(state): State<AppState>,
    Query(query): Query<TelemetryQuery>,
) -> Result<Response, ProxyError> {
    let url = format!("{}/telemetry", state.api_base_url);
    let response = state
        .http_client
        .get(url)
        .query(&[("start", &query.start), ("end", &query.end)])
        .send()
        .await
        .map_err(ProxyError::upstream)?;

    let status = response.status();
    let bytes = response.bytes().await.map_err(ProxyError::upstream)?;
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );

    Ok((status, headers, bytes).into_response())
}

fn render_app_js(refresh_interval_secs: u64, history_window_secs: u64) -> String {
    APP_JS_TEMPLATE
        .replace(
            "__REFRESH_INTERVAL_MS__",
            &(refresh_interval_secs * 1000).to_string(),
        )
        .replace(
            "__HISTORY_WINDOW_MS__",
            &(history_window_secs * 1000).to_string(),
        )
}

struct ProxyError {
    status: StatusCode,
    message: String,
}

impl ProxyError {
    fn upstream(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: error.to_string(),
        }
    }
}

impl IntoResponse for ProxyError {
    fn into_response(self) -> Response {
        (self.status, self.message).into_response()
    }
}

const INDEX_HTML: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>SRM Monitor</title>
  <link rel="icon" href="/favicon.svg" type="image/svg+xml">
  <link rel="stylesheet" href="/styles.css">
</head>
<body>
  <div class="shell">
    <header class="hero">
      <div>
        <p class="eyebrow">SRM Telemetry</p>
        <h1>Live uplink health at a glance.</h1>
        <p class="lede">Responsive browser dashboard for throughput and signal strength, tuned for both desktop and mobile.</p>
      </div>
      <div class="status-card">
        <p class="status-label">Current band</p>
        <p class="status-value" id="band-value">Waiting...</p>
        <p class="status-meta" id="updated-value">No samples yet</p>
      </div>
    </header>

    <section class="metrics">
      <article class="metric-card">
        <span>Signal</span>
        <strong id="signal-value">--%</strong>
      </article>
      <article class="metric-card">
        <span>Rx</span>
        <strong id="rx-value">-- Mbps</strong>
      </article>
      <article class="metric-card">
        <span>Tx</span>
        <strong id="tx-value">-- Mbps</strong>
      </article>
    </section>

    <section class="toolbar">
      <div class="range-card">
        <label class="range-label" for="history-window">History window</label>
        <select id="history-window" class="range-select" aria-label="Select displayed history window">
          <option value="300000">5 minutes</option>
          <option value="3600000">1 hour</option>
          <option value="43200000">12 hours</option>
          <option value="86400000">1 day</option>
          <option value="604800000">1 week</option>
        </select>
      </div>
    </section>

    <section class="charts">
      <article class="chart-card">
        <div class="chart-header">
          <div>
            <p class="chart-title">Throughput</p>
            <p class="chart-subtitle" id="throughput-subtitle">Rx and Tx over the last 12 hours</p>
          </div>
        </div>
        <div class="chart-frame">
          <svg id="throughput-chart" viewBox="0 0 800 320" preserveAspectRatio="none"></svg>
        </div>
      </article>

      <article class="chart-card">
        <div class="chart-header">
          <div>
            <p class="chart-title">Signal Strength</p>
            <p class="chart-subtitle" id="signal-subtitle">Percentage over the last 12 hours</p>
          </div>
        </div>
        <div class="chart-frame">
          <svg id="signal-chart" viewBox="0 0 800 320" preserveAspectRatio="none"></svg>
        </div>
      </article>
    </section>
  </div>
  <script src="/app.js"></script>
</body>
</html>
"##;

const FAVICON_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 128 128">
  <defs>
    <linearGradient id="sky" x1="18" y1="12" x2="112" y2="116" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#1f6fd1"/>
      <stop offset="0.55" stop-color="#17a398"/>
      <stop offset="1" stop-color="#f08a4b"/>
    </linearGradient>
    <linearGradient id="glow" x1="36" y1="24" x2="92" y2="100" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#ffffff" stop-opacity="0.95"/>
      <stop offset="1" stop-color="#fef3de" stop-opacity="0.72"/>
    </linearGradient>
  </defs>
  <rect x="10" y="10" width="108" height="108" rx="28" fill="url(#sky)"/>
  <path d="M31 92c9-25 24-39 44-39 11 0 21 4 31 12" fill="none" stroke="#0c2230" stroke-opacity="0.24" stroke-width="11" stroke-linecap="round"/>
  <path d="M30 90c10-22 24-34 42-34 10 0 20 4 30 11" fill="none" stroke="url(#glow)" stroke-width="8" stroke-linecap="round"/>
  <rect x="30" y="69" width="12" height="25" rx="6" fill="#fff7ea"/>
  <rect x="48" y="57" width="12" height="37" rx="6" fill="#fff7ea"/>
  <rect x="66" y="44" width="12" height="50" rx="6" fill="#fff7ea"/>
  <rect x="84" y="30" width="12" height="64" rx="6" fill="#fff7ea"/>
  <circle cx="95" cy="34" r="11" fill="#fff7ea"/>
  <circle cx="95" cy="34" r="5" fill="#f08a4b"/>
</svg>
"##;

const STYLES_CSS: &str = r##":root {
  color-scheme: light;
  --bg: #f3efe6;
  --panel: rgba(255, 252, 246, 0.9);
  --panel-strong: rgba(255, 250, 240, 0.98);
  --ink: #1d2a33;
  --muted: #596a73;
  --line: rgba(29, 42, 51, 0.08);
  --accent-a: #2176c9;
  --accent-b: #df6d47;
  --accent-c: #169c91;
  --shadow: 0 24px 60px rgba(44, 61, 70, 0.12);
}
* { box-sizing: border-box; }
body {
  margin: 0;
  min-height: 100vh;
  font-family: "Avenir Next", "IBM Plex Sans", system-ui, sans-serif;
  color: var(--ink);
  background:
    radial-gradient(circle at top left, rgba(255,255,255,0.9), transparent 35%),
    linear-gradient(160deg, #efe7d6 0%, #f4f0e8 42%, #e4edf2 100%);
}
.shell {
  max-width: 1240px;
  margin: 0 auto;
  padding: 24px;
}
.hero {
  display: grid;
  grid-template-columns: minmax(0, 1.8fr) minmax(260px, 0.8fr);
  gap: 18px;
  align-items: stretch;
  margin-bottom: 18px;
}
.eyebrow {
  margin: 0 0 10px;
  text-transform: uppercase;
  letter-spacing: 0.16em;
  font-size: 0.75rem;
  color: var(--muted);
}
h1 {
  margin: 0 0 10px;
  font-size: clamp(2rem, 4vw, 4rem);
  line-height: 0.95;
}
.lede {
  margin: 0;
  max-width: 42rem;
  font-size: 1rem;
  line-height: 1.5;
  color: var(--muted);
}
.status-card, .metric-card, .chart-card {
  background: var(--panel);
  border: 1px solid rgba(255,255,255,0.6);
  border-radius: 24px;
  box-shadow: var(--shadow);
  backdrop-filter: blur(10px);
}
.status-card {
  padding: 22px;
  display: flex;
  flex-direction: column;
  justify-content: center;
}
.status-label, .chart-title, .metric-card span {
  margin: 0;
  font-size: 0.85rem;
  text-transform: uppercase;
  letter-spacing: 0.08em;
  color: var(--muted);
}
.status-value {
  margin: 10px 0 8px;
  font-size: clamp(1.75rem, 5vw, 3rem);
  font-weight: 700;
}
.status-meta, .chart-subtitle {
  margin: 0;
  color: var(--muted);
}
.metrics {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 14px;
  margin-bottom: 18px;
}
.toolbar {
  margin-bottom: 18px;
}
.metric-card {
  padding: 18px 20px;
}
.range-card {
  display: inline-flex;
  flex-wrap: wrap;
  gap: 12px;
  align-items: center;
  padding: 14px 18px;
  background: var(--panel);
  border: 1px solid rgba(255,255,255,0.6);
  border-radius: 20px;
  box-shadow: var(--shadow);
  backdrop-filter: blur(10px);
}
.range-label {
  font-size: 0.85rem;
  text-transform: uppercase;
  letter-spacing: 0.08em;
  color: var(--muted);
}
.range-select {
  appearance: none;
  border: 1px solid var(--line);
  border-radius: 999px;
  background: var(--panel-strong);
  color: var(--ink);
  font: inherit;
  padding: 10px 16px;
  min-width: 160px;
}
.range-select:focus {
  outline: 2px solid rgba(33, 118, 201, 0.35);
  outline-offset: 2px;
}
.metric-card strong {
  display: block;
  margin-top: 10px;
  font-size: clamp(1.6rem, 3vw, 2.4rem);
}
.charts {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 18px;
}
.chart-card {
  padding: 18px;
}
.chart-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 12px;
}
.chart-frame {
  background: var(--panel-strong);
  border-radius: 20px;
  border: 1px solid var(--line);
  overflow: hidden;
  min-height: 320px;
}
svg {
  display: block;
  width: 100%;
  height: 320px;
}
.chart-grid-line {
  stroke: var(--line);
  stroke-width: 1;
}
.chart-axis-label {
  fill: var(--muted);
  font-size: 13px;
  font-family: inherit;
}
.chart-path-rx, .chart-path-tx, .chart-path-signal {
  fill: none;
  stroke-linecap: round;
  stroke-linejoin: round;
  stroke-width: 3.5;
}
.chart-path-rx { stroke: var(--accent-a); }
.chart-path-tx { stroke: var(--accent-b); }
.chart-path-signal { stroke: var(--accent-c); }
.chart-empty {
  fill: var(--muted);
  font-size: 20px;
  text-anchor: middle;
}
@media (max-width: 920px) {
  .hero, .charts { grid-template-columns: 1fr; }
}
@media (max-width: 640px) {
  .shell { padding: 16px; }
  .metrics { grid-template-columns: 1fr; }
  .range-card { width: 100%; }
  .range-select { width: 100%; }
  .chart-card { padding: 14px; }
  .chart-frame, svg { min-height: 260px; height: 260px; }
}
"##;

const APP_JS_TEMPLATE: &str = r##"const refreshIntervalMs = __REFRESH_INTERVAL_MS__;
const defaultHistoryWindowMs = __HISTORY_WINDOW_MS__;
const historyWindowOptions = [
  { valueMs: 5 * 60 * 1000, label: '5 minutes' },
  { valueMs: 60 * 60 * 1000, label: '1 hour' },
  { valueMs: 12 * 60 * 60 * 1000, label: '12 hours' },
  { valueMs: 24 * 60 * 60 * 1000, label: '1 day' },
  { valueMs: 7 * 24 * 60 * 60 * 1000, label: '1 week' },
];

const dom = {
  band: document.getElementById('band-value'),
  updated: document.getElementById('updated-value'),
  signal: document.getElementById('signal-value'),
  rx: document.getElementById('rx-value'),
  tx: document.getElementById('tx-value'),
  historyWindow: document.getElementById('history-window'),
  throughputSubtitle: document.getElementById('throughput-subtitle'),
  signalSubtitle: document.getElementById('signal-subtitle'),
  throughput: document.getElementById('throughput-chart'),
  signalChart: document.getElementById('signal-chart'),
};

const throughputMax = 2000;
const signalMax = 105;
let selectedHistoryWindowMs = normalizeHistoryWindow(defaultHistoryWindowMs);

function normalizeHistoryWindow(windowMs) {
  const match = historyWindowOptions.find(option => option.valueMs === windowMs);
  return match ? match.valueMs : 12 * 60 * 60 * 1000;
}

function selectedHistoryWindow() {
  return historyWindowOptions.find(option => option.valueMs === selectedHistoryWindowMs)
    ?? historyWindowOptions[2];
}

function syncHistoryWindowControl() {
  dom.historyWindow.value = String(selectedHistoryWindowMs);
}

function updateRangeCopy() {
  const option = selectedHistoryWindow();
  dom.throughputSubtitle.textContent = `Rx and Tx over the last ${option.label.toLowerCase()}`;
  dom.signalSubtitle.textContent = `Percentage over the last ${option.label.toLowerCase()}`;
}

async function fetchTelemetry() {
  const end = new Date();
  const start = new Date(end.getTime() - selectedHistoryWindowMs);
  const params = new URLSearchParams({ start: start.toISOString(), end: end.toISOString() });
  const response = await fetch(`/api/telemetry?${params.toString()}`);
  if (!response.ok) {
    throw new Error(`Request failed with status ${response.status}`);
  }
  return response.json();
}

function formatMbps(bps) {
  return `${(bps / 1000000).toFixed(3)} Mbps`;
}

function formatLocalTime(iso) {
  return new Intl.DateTimeFormat([], {
    hour: '2-digit', minute: '2-digit', second: '2-digit'
  }).format(new Date(iso));
}

function setSummary(samples) {
  const latest = samples[samples.length - 1];
  if (!latest) {
    dom.band.textContent = 'Waiting...';
    dom.updated.textContent = 'No samples yet';
    dom.signal.textContent = '--%';
    dom.rx.textContent = '-- Mbps';
    dom.tx.textContent = '-- Mbps';
    return;
  }

  dom.band.textContent = latest.band;
  dom.updated.textContent = `Updated ${formatLocalTime(latest.timestamp_utc)}`;
  dom.signal.textContent = `${latest.signal_strength}%`;
  dom.rx.textContent = formatMbps(latest.rx_bps);
  dom.tx.textContent = formatMbps(latest.tx_bps);
}

function buildLinePath(samples, yAccessor, yMax) {
  if (samples.length === 0) return '';
  const width = 800;
  const height = 320;
  const padding = { top: 18, right: 18, bottom: 34, left: 52 };
  const minX = new Date(samples[0].timestamp_utc).getTime();
  const maxX = new Date(samples[samples.length - 1].timestamp_utc).getTime();
  const safeMaxX = Math.max(maxX, minX + 1000);
  const plotWidth = width - padding.left - padding.right;
  const plotHeight = height - padding.top - padding.bottom;

  return samples.map((sample, index) => {
    const timestamp = new Date(sample.timestamp_utc).getTime();
    const x = padding.left + ((timestamp - minX) / (safeMaxX - minX)) * plotWidth;
    const y = padding.top + (1 - (Math.min(yAccessor(sample), yMax) / yMax)) * plotHeight;
    return `${index === 0 ? 'M' : 'L'}${x.toFixed(2)},${y.toFixed(2)}`;
  }).join(' ');
}

function buildGrid(yMax, width = 800, height = 320) {
  const padding = { top: 18, right: 18, bottom: 34, left: 52 };
  const plotWidth = width - padding.left - padding.right;
  const plotHeight = height - padding.top - padding.bottom;
  const rows = 4;
  let markup = '';
  for (let index = 0; index <= rows; index += 1) {
    const y = padding.top + (plotHeight / rows) * index;
    const value = ((rows - index) / rows) * yMax;
    markup += `<line class="chart-grid-line" x1="${padding.left}" y1="${y}" x2="${padding.left + plotWidth}" y2="${y}" />`;
    markup += `<text class="chart-axis-label" x="10" y="${y + 4}">${value.toFixed(0)}</text>`;
  }
  return markup;
}

function formatAxisTime(timestamp) {
  if (selectedHistoryWindowMs > 24 * 60 * 60 * 1000) {
    return new Intl.DateTimeFormat([], {
      weekday: 'short',
      day: 'numeric',
      month: 'short',
    }).format(timestamp);
  }

  if (selectedHistoryWindowMs > 12 * 60 * 60 * 1000) {
    return new Intl.DateTimeFormat([], {
      day: 'numeric',
      month: 'short',
      hour: '2-digit',
      minute: '2-digit',
    }).format(timestamp);
  }

  return new Intl.DateTimeFormat([], {
    hour: '2-digit',
    minute: '2-digit',
  }).format(timestamp);
}

function buildTimeLabels(samples, width = 800, height = 320) {
  if (samples.length === 0) return '';
  const padding = { top: 18, right: 18, bottom: 34, left: 52 };
  const minX = new Date(samples[0].timestamp_utc).getTime();
  const maxX = new Date(samples[samples.length - 1].timestamp_utc).getTime();
  const safeMaxX = Math.max(maxX, minX + 1000);
  const plotWidth = width - padding.left - padding.right;
  const labels = 4;
  let markup = '';
  for (let index = 0; index <= labels; index += 1) {
    const ratio = index / labels;
    const x = padding.left + ratio * plotWidth;
    const timestamp = new Date(minX + ratio * (safeMaxX - minX));
    markup += `<text class="chart-axis-label" x="${x}" y="${height - 10}" text-anchor="middle">${formatAxisTime(timestamp)}</text>`;
  }
  return markup;
}

function renderChart(svg, samples, lines, yMax, emptyLabel) {
  const grid = buildGrid(yMax);
  const labels = buildTimeLabels(samples);
  if (samples.length === 0) {
    svg.innerHTML = `${grid}<text class="chart-empty" x="400" y="170">${emptyLabel}</text>`;
    return;
  }

  const paths = lines.map(({ accessor, className }) => {
    const path = buildLinePath(samples, accessor, yMax);
    return `<path class="${className}" d="${path}" />`;
  }).join('');

  svg.innerHTML = `${grid}${paths}${labels}`;
}

function render(samples) {
  setSummary(samples);
  renderChart(dom.throughput, samples, [
    { accessor: sample => sample.rx_bps / 1000000, className: 'chart-path-rx' },
    { accessor: sample => sample.tx_bps / 1000000, className: 'chart-path-tx' },
  ], throughputMax, 'Waiting for throughput samples');

  renderChart(dom.signalChart, samples, [
    { accessor: sample => sample.signal_strength, className: 'chart-path-signal' },
  ], signalMax, 'Waiting for signal samples');
}

async function refresh() {
  try {
    const samples = await fetchTelemetry();
    render(samples);
  } catch (error) {
    dom.updated.textContent = error.message;
  }
}

dom.historyWindow.addEventListener('change', async event => {
  selectedHistoryWindowMs = normalizeHistoryWindow(Number(event.target.value));
  syncHistoryWindowControl();
  updateRangeCopy();
  await refresh();
});

syncHistoryWindowControl();
updateRangeCopy();
refresh();
setInterval(refresh, refreshIntervalMs);
"##;

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Json;
    use axum::body;
    use axum::body::Body;
    use axum::http::Request;
    use serde_json::Value;
    use std::net::SocketAddr;
    use tower::util::ServiceExt;

    fn test_config(
        base_url: &str,
        refresh_interval_secs: u64,
        history_window_secs: u64,
    ) -> WebConfig {
        WebConfig {
            server: srm_common::config::WebServerSettings {
                bind_address: "127.0.0.1:6080".to_string(),
            },
            api: srm_common::config::WebApiSettings {
                base_url: base_url.to_string(),
                refresh_interval_secs,
                history_window_secs,
            },
        }
    }

    async fn spawn_upstream(status: StatusCode) -> SocketAddr {
        let app = Router::new().route(
            "/telemetry",
            get(move || async move {
                let sample = serde_json::json!([{
                  "timestamp_utc": "2026-03-15T18:44:12Z",
                  "band": "5G-1",
                  "signal_strength": 78,
                  "rx_bps": 800000000,
                  "tx_bps": 720000000
                }]);
                (status, Json(sample))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();

        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        address
    }

    #[test]
    fn rendered_app_js_injects_refresh_and_history_defaults() {
        let script = render_app_js(30, 12 * 60 * 60);

        assert!(script.contains("const refreshIntervalMs = 30000;"));
        assert!(script.contains("const defaultHistoryWindowMs = 43200000;"));
    }

    #[test]
    fn rendered_app_js_contains_all_history_window_options() {
        let script = render_app_js(30, 12 * 60 * 60);

        for label in ["5 minutes", "1 hour", "12 hours", "1 day", "1 week"] {
            assert!(script.contains(label), "missing option {label}");
        }
    }

    #[test]
    fn rendered_app_js_defaults_unknown_history_window_to_twelve_hours() {
        let script = render_app_js(30, 42);

        assert!(script.contains("return match ? match.valueMs : 12 * 60 * 60 * 1000;"));
    }

    #[test]
    fn index_html_exposes_history_window_selector_and_default_copy() {
        assert!(INDEX_HTML.contains("id=\"history-window\""));
        assert!(INDEX_HTML.contains("rel=\"icon\" href=\"/favicon.svg\""));
        assert!(INDEX_HTML.contains("Rx and Tx over the last 12 hours"));
        assert!(INDEX_HTML.contains("Percentage over the last 12 hours"));
    }

    #[test]
    fn build_state_trims_base_url_and_clamps_intervals() {
        let state = build_state(test_config("http://127.0.0.1:6081/", 0, 1));

        assert_eq!(state.api_base_url, "http://127.0.0.1:6081");
        assert_eq!(state.refresh_interval_secs, 1);
        assert_eq!(state.history_window_secs, 60);
    }

    #[tokio::test]
    async fn index_route_serves_html() {
        let app = build_app(build_state(test_config("http://127.0.0.1:6081", 30, 43200)));
        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();

        assert_eq!(status, StatusCode::OK);
        assert!(
            std::str::from_utf8(&body)
                .unwrap()
                .contains("History window")
        );
    }

    #[tokio::test]
    async fn app_js_route_uses_configured_history_window() {
        let app = build_app(build_state(test_config("http://127.0.0.1:6081", 30, 43200)));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/app.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let script = std::str::from_utf8(&body).unwrap();

        assert!(script.contains("const defaultHistoryWindowMs = 43200000;"));
    }

    #[tokio::test]
    async fn favicon_route_serves_svg() {
        let app = build_app(build_state(test_config("http://127.0.0.1:6081", 30, 43200)));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/favicon.svg")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();

        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            headers.get(header::CONTENT_TYPE).unwrap(),
            "image/svg+xml; charset=utf-8"
        );
        assert!(std::str::from_utf8(&body).unwrap().contains("<svg"));
    }

    #[tokio::test]
    async fn proxy_route_forwards_upstream_json() {
        let upstream = spawn_upstream(StatusCode::OK).await;
        let app = build_app(build_state(test_config(
            &format!("http://{upstream}"),
            30,
            43200,
        )));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/telemetry?start=2026-03-15T18:00:00Z&end=2026-03-15T19:00:00Z")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            headers.get(header::CONTENT_TYPE).unwrap(),
            "application/json; charset=utf-8"
        );
        assert!(payload.as_array().is_some_and(|items| items.len() == 1));
    }

    #[tokio::test]
    async fn proxy_route_preserves_upstream_error_status() {
        let upstream = spawn_upstream(StatusCode::BAD_GATEWAY).await;
        let app = build_app(build_state(test_config(
            &format!("http://{upstream}"),
            30,
            43200,
        )));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/telemetry?start=2026-03-15T18:00:00Z&end=2026-03-15T19:00:00Z")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    }
}
