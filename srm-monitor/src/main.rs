mod graph;
use anyhow::{Context, Result};
use chrono::{DateTime, TimeDelta, Utc};
use srm_common::config::{ApiClientSettings, GuiConfig, env_or_manifest_path, load_toml_file};
use srm_common::models::TelemetrySample;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

const APP_NAME: &str = env!("CARGO_PKG_NAME");
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const CONFIG_ENV_VAR: &str = "SRM_GRAPH_GUI_CONFIG";
const DEFAULT_CONFIG_PATH: &str = "config/gui.toml";
const INITIAL_HISTORY_WINDOW_SECS: i64 = 5 * 60;

fn run() -> Result<()> {
    println!("{} v{}", APP_NAME, APP_VERSION);
    let config_path = env_or_manifest_path(
        CONFIG_ENV_VAR,
        DEFAULT_CONFIG_PATH,
        env!("CARGO_MANIFEST_DIR"),
    );
    let config: GuiConfig = load_toml_file(&config_path)?;
    let (event_sender, event_receiver) = std::sync::mpsc::channel();
    let (command_sender, command_receiver) = std::sync::mpsc::channel();
    let history_start = parse_history_start(&config.api.history_start)?;

    spawn_api_poller(config.api, event_sender, command_receiver, history_start)?;
    graph::run_monitor_window(
        APP_NAME,
        APP_VERSION,
        event_receiver,
        command_sender,
        history_start,
    )
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error=fatal details={}", err);
        std::process::exit(1);
    }
}

fn spawn_api_poller(
    config: ApiClientSettings,
    sender: Sender<graph::GraphEvent>,
    command_receiver: Receiver<graph::GraphCommand>,
    history_start: DateTime<Utc>,
) -> Result<()> {
    thread::Builder::new()
        .name("srm-api-poller".to_string())
        .spawn(move || poll_api_loop(config, sender, command_receiver, history_start))
        .context("failed to spawn API poller thread")?;
    Ok(())
}

fn poll_api_loop(
    config: ApiClientSettings,
    sender: Sender<graph::GraphEvent>,
    command_receiver: Receiver<graph::GraphCommand>,
    history_start: DateTime<Utc>,
) {
    let mut follow_latest = true;

    if load_live_window(&config.base_url, &sender, history_start).is_err() {
        return;
    }

    loop {
        if follow_latest {
            match command_receiver
                .recv_timeout(Duration::from_secs(config.refresh_interval_secs.max(1)))
            {
                Ok(graph::GraphCommand::FollowLatest) => {
                    if load_live_window(&config.base_url, &sender, history_start).is_err() {
                        return;
                    }
                }
                Ok(graph::GraphCommand::LoadVisibleRange { start, end }) => {
                    follow_latest = false;
                    if load_visible_range(&config.base_url, &sender, history_start, start, end)
                        .is_err()
                    {
                        return;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    if load_live_window(&config.base_url, &sender, history_start).is_err() {
                        return;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => return,
            }
        } else {
            match command_receiver.recv() {
                Ok(graph::GraphCommand::FollowLatest) => {
                    follow_latest = true;
                    if load_live_window(&config.base_url, &sender, history_start).is_err() {
                        return;
                    }
                }
                Ok(graph::GraphCommand::LoadVisibleRange { start, end }) => {
                    if load_visible_range(&config.base_url, &sender, history_start, start, end)
                        .is_err()
                    {
                        return;
                    }
                }
                Err(_) => return,
            }
        }
    }
}

fn load_live_window(
    base_url: &str,
    sender: &Sender<graph::GraphEvent>,
    history_start: DateTime<Utc>,
) -> Result<()> {
    let end = Utc::now();
    let start = initial_history_start(history_start, end);
    load_range(base_url, sender, start, end)
}

fn load_visible_range(
    base_url: &str,
    sender: &Sender<graph::GraphEvent>,
    history_start: DateTime<Utc>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<()> {
    let now = Utc::now();
    let (start, end) = normalize_requested_range(history_start, start, end, now);
    load_range(base_url, sender, start, end)
}

fn load_range(
    base_url: &str,
    sender: &Sender<graph::GraphEvent>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<()> {
    match fetch_samples(base_url, start, end) {
        Ok(samples) => sender
            .send(graph::GraphEvent::ReplaceHistory(samples))
            .map_err(|error| anyhow::anyhow!(error.to_string())),
        Err(error) => sender
            .send(graph::GraphEvent::Error(error.to_string()))
            .map_err(|send_error| anyhow::anyhow!(send_error.to_string())),
    }
}

fn fetch_samples(
    base_url: &str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<TelemetrySample>> {
    let url = format!("{}/telemetry", base_url.trim_end_matches('/'));
    let mut response = ureq::get(&url)
        .query("start", start.to_rfc3339())
        .query("end", end.to_rfc3339())
        .call()?;

    Ok(response.body_mut().read_json()?)
}

fn parse_history_start(value: &str) -> Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(value)?.with_timezone(&Utc))
}

fn initial_history_start(history_start: DateTime<Utc>, now: DateTime<Utc>) -> DateTime<Utc> {
    let rolling_start = now - TimeDelta::seconds(INITIAL_HISTORY_WINDOW_SECS);
    if history_start > rolling_start {
        history_start
    } else {
        rolling_start
    }
}

fn normalize_requested_range(
    history_start: DateTime<Utc>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    now: DateTime<Utc>,
) -> (DateTime<Utc>, DateTime<Utc>) {
    let start = start.max(history_start);
    let mut end = end.min(now);
    if end <= start {
        end = (start + TimeDelta::seconds(1)).min(now);
    }
    (start, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_history_uses_last_five_minutes_when_available() {
        let now = DateTime::parse_from_rfc3339("2026-03-15T15:10:00+00:00")
            .unwrap()
            .with_timezone(&Utc);
        let history_start = DateTime::parse_from_rfc3339("2026-03-15T14:00:00+00:00")
            .unwrap()
            .with_timezone(&Utc);

        let start = initial_history_start(history_start, now);

        assert_eq!(start, now - TimeDelta::minutes(5));
    }

    #[test]
    fn initial_history_respects_configured_floor() {
        let now = DateTime::parse_from_rfc3339("2026-03-15T15:10:00+00:00")
            .unwrap()
            .with_timezone(&Utc);
        let history_start = DateTime::parse_from_rfc3339("2026-03-15T15:08:00+00:00")
            .unwrap()
            .with_timezone(&Utc);

        let start = initial_history_start(history_start, now);

        assert_eq!(start, history_start);
    }

    #[test]
    fn normalize_requested_range_clamps_to_history_floor_and_now() {
        let history_start = DateTime::parse_from_rfc3339("2026-03-15T15:00:00+00:00")
            .unwrap()
            .with_timezone(&Utc);
        let now = DateTime::parse_from_rfc3339("2026-03-15T15:10:00+00:00")
            .unwrap()
            .with_timezone(&Utc);
        let start = DateTime::parse_from_rfc3339("2026-03-15T14:00:00+00:00")
            .unwrap()
            .with_timezone(&Utc);
        let end = DateTime::parse_from_rfc3339("2026-03-15T15:20:00+00:00")
            .unwrap()
            .with_timezone(&Utc);

        let (start, end) = normalize_requested_range(history_start, start, end, now);

        assert_eq!(start, history_start);
        assert_eq!(end, now);
    }
}
