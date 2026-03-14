mod synology;
use anyhow::Result;
use chrono::{DateTime, Datelike, Utc};
use chrono_tz::Tz;
use serde::Deserialize;
use std::fs::OpenOptions;
use std::io::Write;
use std::str::FromStr;
use std::thread;
use std::time::Duration;

const CSV_FILE_PATH: &str = "avg_rates.csv";
const NODE_ID: i32 = 8;
const POLL_INTERVAL_SECS: u64 = 30;
const TIMESTAMP_FIELD_WIDTH: usize = 25;
const BAND_FIELD_WIDTH: usize = 4;

#[derive(Deserialize)]
struct TomlCredentials {
    credentials: Credentials,
}

#[derive(Deserialize)]
struct Credentials {
    username: String,
    password: String,
}

fn read_credentials() -> Result<Credentials> {
    let s = std::fs::read_to_string("secrets/srm_login.toml")?;
    let cfg: TomlCredentials = toml::from_str(&s)?;
    Ok(cfg.credentials)
}

fn format_bps(rate_bps: u64) -> String {
    let units = ["bps", "Kbps", "Mbps", "Gbps", "Tbps"];
    let mut value = rate_bps as f64;
    let mut unit_idx = 0usize;

    while value >= 1000.0 && unit_idx < units.len() - 1 {
        value /= 1000.0;
        unit_idx += 1;
    }

    if unit_idx == 0 {
        format!("{} {}", rate_bps, units[unit_idx])
    } else {
        format!("{:.3} {}", value, units[unit_idx])
    }
}

fn open_csv_file() -> Result<std::fs::File> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(CSV_FILE_PATH)?;

    if file.metadata()?.len() == 0 {
        writeln!(file, "timestamp,band,avg_rx_bps,avg_tx_bps")?;
    }

    Ok(file)
}

fn local_timezone() -> Result<Tz> {
    let timezone_name = iana_time_zone::get_timezone()?;
    Ok(Tz::from_str(&timezone_name)?)
}

fn iso8601_timestamp(now: DateTime<Tz>) -> String {
    now.format("%Y-%m-%dT%H:%M:%S%:z").to_string()
}

fn day_suffix(day: u32) -> &'static str {
    match day {
        11..=13 => "th",
        _ => match day % 10 {
            1 => "st",
            2 => "nd",
            3 => "rd",
            _ => "th",
        },
    }
}

fn console_timestamp(now: DateTime<Tz>) -> String {
    let day = now.day();
    format!(
        "{} {:>2}{} {} {}",
        now.format("%a"),
        day,
        day_suffix(day),
        now.format("%b %Y %H:%M"),
        now.format("%Z")
    )
}

fn append_sample(
    file: &mut std::fs::File,
    timestamp: &str,
    band: &str,
    rx_bps: u64,
    tx_bps: u64,
) -> Result<()> {
    writeln!(
        file,
        "{},{},{},{}",
        timestamp,
        band,
        rx_bps,
        tx_bps
    )?;
    file.flush()?;
    Ok(())
}

fn run() -> Result<()> {
    let creds = read_credentials()?;
    let timezone = local_timezone()?;
    let mut csv_file = open_csv_file()?;
    let mut last_band: Option<String> = None;
    let mut synology: Option<synology::Synology> = None;

    loop {
        if synology.is_none() {
            match synology::Synology::new(&creds.username, &creds.password) {
                Ok(session) => synology = Some(session),
                Err(err) => {
                    eprintln!("error=login_failed details={}", err);
                    thread::sleep(Duration::from_secs(POLL_INTERVAL_SECS));
                    continue;
                }
            }
        }

        let Some(session) = synology.as_ref() else {
            thread::sleep(Duration::from_secs(POLL_INTERVAL_SECS));
            continue;
        };

        match session.fetch_avg_rates(NODE_ID) {
            Ok((band, rx_bps, tx_bps)) => {
                let now = Utc::now().with_timezone(&timezone);
                let csv_timestamp = iso8601_timestamp(now);

                append_sample(&mut csv_file, &csv_timestamp, &band, rx_bps, tx_bps)?;

                if last_band.as_deref() != Some(band.as_str()) {
                    println!(
                        "{:<TIMESTAMP_FIELD_WIDTH$} band={:<BAND_FIELD_WIDTH$} tx={} rx={}",
                        console_timestamp(now),
                        band,
                        format_bps(tx_bps),
                        format_bps(rx_bps)
                    );
                    last_band = Some(band);
                }
            }
            Err(err) => {
                eprintln!("error=fetch_failed details={}", err);
                synology = None;
            }
        }

        thread::sleep(Duration::from_secs(POLL_INTERVAL_SECS));
    }
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error=fatal details={}", err);
        std::process::exit(1);
    }
}
