use anyhow::Result;
use chrono::Utc;
use mongodb::Client;
use srm_common::config::{ServiceConfig, env_or_manifest_path, load_toml_file};
use srm_common::models::{MongoTelemetryRecord, TelemetrySample, ensure_telemetry_indexes};
use srm_common::synology::Synology;
use tokio::time::{Duration, sleep};

const APP_NAME: &str = env!("CARGO_PKG_NAME");
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const CONFIG_ENV_VAR: &str = "SRM_MONITOR_SERVICE_CONFIG";
const DEFAULT_CONFIG_PATH: &str = "config/service.toml";

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
    let config: ServiceConfig = load_toml_file(&config_path)?;

    let client = Client::with_uri_str(&config.mongodb.url).await?;
    let collection = client
        .database(&config.mongodb.database)
        .collection::<MongoTelemetryRecord>(&config.mongodb.collection);
    ensure_telemetry_indexes(&collection).await?;

    let mut last_band: Option<String> = None;
    let mut synology: Option<Synology> = None;

    loop {
        if synology.is_none() {
            match Synology::new(
                &config.synology.base_url,
                &config.synology.credentials.username,
                &config.synology.credentials.password,
            ) {
                Ok(session) => synology = Some(session),
                Err(error) => {
                    eprintln!("error=login_failed details={}", error);
                    if wait_for_shutdown(config.synology.poll_interval_secs).await {
                        break;
                    }
                    continue;
                }
            }
        }

        if let Some(session) = synology.as_ref() {
            match session.fetch_avg_rates(config.synology.node_id) {
                Ok((band, signal_strength, rx_bps, tx_bps)) => {
                    let sample = TelemetrySample::new(
                        Utc::now(),
                        band.clone(),
                        signal_strength,
                        rx_bps,
                        tx_bps,
                    );

                    collection
                        .insert_one(MongoTelemetryRecord::from(&sample))
                        .await?;
                    print_band_change(&sample, &mut last_band);
                }
                Err(error) => {
                    eprintln!("error=fetch_failed details={}", error);
                    synology = None;
                }
            }
        }

        if wait_for_shutdown(config.synology.poll_interval_secs).await {
            break;
        }
    }

    Ok(())
}

async fn wait_for_shutdown(poll_interval_secs: u64) -> bool {
    tokio::select! {
        _ = tokio::signal::ctrl_c() => true,
        _ = sleep(Duration::from_secs(poll_interval_secs.max(1))) => false,
    }
}

fn print_band_change(sample: &TelemetrySample, last_band: &mut Option<String>) {
    if last_band.as_deref() == Some(sample.band.as_str()) {
        return;
    }

    println!(
        "timestamp={} band={} signalstrength={} tx={} rx={}",
        sample.timestamp_utc.to_rfc3339(),
        sample.band,
        sample.signal_strength,
        format_bps(sample.tx_bps),
        format_bps(sample.rx_bps),
    );
    *last_band = Some(sample.band.clone());
}

fn format_bps(rate_bps: u64) -> String {
    let units = ["bps", "Kbps", "Mbps", "Gbps", "Tbps"];
    let mut value = rate_bps as f64;
    let mut unit_index = 0usize;

    while value >= 1000.0 && unit_index < units.len() - 1 {
        value /= 1000.0;
        unit_index += 1;
    }

    if unit_index == 0 {
        format!("{} {}", rate_bps, units[unit_index])
    } else {
        format!("{:.3} {}", value, units[unit_index])
    }
}
