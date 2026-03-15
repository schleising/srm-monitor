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

    let state = AppState {
        http_client: Client::new(),
        api_base_url: config.api.base_url.trim_end_matches('/').to_string(),
        refresh_interval_secs: config.api.refresh_interval_secs.max(1),
        history_window_secs: config.api.history_window_secs.max(60),
    };

    let app = Router::new()
        .route("/", get(index))
        .route("/app.js", get(app_js))
        .route("/styles.css", get(styles_css))
        .route("/api/telemetry", get(proxy_telemetry))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&config.server.bind_address).await?;
    println!("listening=http://{}", config.server.bind_address);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
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

    <section class="charts">
      <article class="chart-card">
        <div class="chart-header">
          <div>
            <p class="chart-title">Throughput</p>
            <p class="chart-subtitle">Rx and Tx over the last five minutes</p>
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
            <p class="chart-subtitle">Percentage over the last five minutes</p>
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
.metric-card {
  padding: 18px 20px;
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
  .chart-card { padding: 14px; }
  .chart-frame, svg { min-height: 260px; height: 260px; }
}
"##;

const APP_JS_TEMPLATE: &str = r##"const refreshIntervalMs = __REFRESH_INTERVAL_MS__;
const historyWindowMs = __HISTORY_WINDOW_MS__;

const dom = {
  band: document.getElementById('band-value'),
  updated: document.getElementById('updated-value'),
  signal: document.getElementById('signal-value'),
  rx: document.getElementById('rx-value'),
  tx: document.getElementById('tx-value'),
  throughput: document.getElementById('throughput-chart'),
  signalChart: document.getElementById('signal-chart'),
};

const throughputMax = 2000;
const signalMax = 105;

async function fetchTelemetry() {
  const end = new Date();
  const start = new Date(end.getTime() - historyWindowMs);
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
    markup += `<text class="chart-axis-label" x="${x}" y="${height - 10}" text-anchor="middle">${new Intl.DateTimeFormat([], { hour: '2-digit', minute: '2-digit' }).format(timestamp)}</text>`;
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

refresh();
setInterval(refresh, refreshIntervalMs);
"##;
