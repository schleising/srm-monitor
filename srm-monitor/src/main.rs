mod graph;
mod profiling;
use anyhow::{Context, Result};
use chrono::{DateTime, TimeDelta, Utc};
use srm_common::config::{ApiClientSettings, GuiConfig, env_or_manifest_path, load_toml_file};
use srm_common::models::TelemetrySample;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

const APP_NAME: &str = env!("CARGO_PKG_NAME");
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const CONFIG_ENV_VAR: &str = "SRM_GRAPH_GUI_CONFIG";
const DEFAULT_CONFIG_PATH: &str = "config/gui.toml";

fn run() -> Result<()> {
    let _profiling_session = profiling::init_from_env()?;
    println!("{} v{}", APP_NAME, APP_VERSION);
    let config_path = env_or_manifest_path(
        CONFIG_ENV_VAR,
        DEFAULT_CONFIG_PATH,
        env!("CARGO_MANIFEST_DIR"),
    );
    let config: GuiConfig = load_toml_file(&config_path)?;
    let (event_sender, event_receiver) = std::sync::mpsc::channel();

    spawn_api_poller(config.api, event_sender)?;
    graph::run_monitor_window(APP_NAME, APP_VERSION, event_receiver)
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error=fatal details={}", err);
        std::process::exit(1);
    }
}

fn spawn_api_poller(config: ApiClientSettings, sender: Sender<graph::GraphEvent>) -> Result<()> {
    thread::Builder::new()
        .name("srm-api-poller".to_string())
        .spawn(move || poll_api_loop(config, sender))
        .context("failed to spawn API poller thread")?;
    Ok(())
}

fn poll_api_loop(config: ApiClientSettings, sender: Sender<graph::GraphEvent>) {
    let mut next_start = match parse_history_start(&config.history_start) {
        Ok(timestamp) => timestamp,
        Err(error) => {
            let _ = sender.send(graph::GraphEvent::Error(error.to_string()));
            return;
        }
    };
    let mut replace_history = true;

    loop {
        let end = Utc::now();
        match fetch_samples(&config.base_url, next_start, end) {
            Ok(samples) => {
                let last_timestamp = samples.last().map(|sample| sample.timestamp_utc);

                if replace_history {
                    if sender
                        .send(graph::GraphEvent::ReplaceHistory(samples))
                        .is_err()
                    {
                        break;
                    }
                    replace_history = false;
                } else if !samples.is_empty()
                    && sender
                        .send(graph::GraphEvent::AppendSamples(samples))
                        .is_err()
                {
                    break;
                }

                if let Some(timestamp) = last_timestamp {
                    next_start = timestamp + TimeDelta::milliseconds(1);
                }
            }
            Err(error) => {
                if sender
                    .send(graph::GraphEvent::Error(error.to_string()))
                    .is_err()
                {
                    break;
                }
            }
        }

        thread::sleep(Duration::from_secs(config.refresh_interval_secs.max(1)));
    }
}

fn fetch_samples(
    base_url: &str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<TelemetrySample>> {
    let url = format!("{}/telemetry", base_url.trim_end_matches('/'));
    let mut response = ureq::get(&url)
        .query("start", &start.to_rfc3339())
        .query("end", &end.to_rfc3339())
        .call()?;

    Ok(response.body_mut().read_json()?)
}

fn parse_history_start(value: &str) -> Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(value)?.with_timezone(&Utc))
}
