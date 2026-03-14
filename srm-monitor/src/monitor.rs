use crate::synology::Synology;
use anyhow::Result;
use chrono::{DateTime, Datelike, Utc};
use chrono_tz::Tz;
use signal_hook::consts::signal::{SIGINT, SIGTERM};
use signal_hook::flag;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

const CSV_FILE_PATH: &str = "avg_rates.csv";
const NODE_ID: i32 = 8;
const POLL_INTERVAL_SECS: u64 = 30;
const TIMESTAMP_FIELD_WIDTH: usize = 25;
const BAND_FIELD_WIDTH: usize = 4;
const SLEEP_SLICE_MILLIS: u64 = 250;

pub struct Monitor {
    timezone: Tz,
    received_signal: Arc<AtomicUsize>,
    csv_file: File,
    last_band: Option<String>,
    synology: Option<Synology>,
}

impl Monitor {
    pub fn new() -> Result<Self> {
        Ok(Self {
            timezone: local_timezone()?,
            received_signal: install_shutdown_handlers()?,
            csv_file: open_csv_file()?,
            last_band: None,
            synology: None,
        })
    }

    pub fn run(&mut self, username: &str, password: &str) -> Result<()> {
        loop {
            if self.handle_shutdown() {
                return Ok(());
            }

            if !self.ensure_session(username, password) {
                continue;
            }

            self.poll_once()?;

            if self.sleep_until_next_poll() {
                continue;
            }
        }
    }

    fn handle_shutdown(&mut self) -> bool {
        let signal = self.received_signal.load(Ordering::Relaxed);
        if signal == 0 {
            return false;
        }

        println!(
            "shutdown signal={} action=cleanup",
            shutdown_signal_name(signal)
        );
        drop(self.synology.take());
        println!(
            "shutdown signal={} action=exit",
            shutdown_signal_name(signal)
        );
        true
    }

    fn ensure_session(&mut self, username: &str, password: &str) -> bool {
        if self.synology.is_some() {
            return true;
        }

        match Synology::new(username, password) {
            Ok(session) => {
                self.synology = Some(session);
                true
            }
            Err(err) => {
                eprintln!("error=login_failed details={}", err);
                // Back off once after a failed login; the caller skips the normal end-of-loop sleep.
                self.sleep_until_next_poll();
                false
            }
        }
    }

    fn poll_once(&mut self) -> Result<()> {
        let fetch_result = {
            let Some(session) = self.synology.as_ref() else {
                return Ok(());
            };
            session.fetch_avg_rates(NODE_ID)
        };

        match fetch_result {
            Ok((band, rx_bps, tx_bps)) => {
                let now = Utc::now().with_timezone(&self.timezone);
                let csv_timestamp = iso8601_timestamp(now);

                append_sample(&mut self.csv_file, &csv_timestamp, &band, rx_bps, tx_bps)?;
                self.print_band_change(now, &band, rx_bps, tx_bps);
            }
            Err(err) => {
                eprintln!("error=fetch_failed details={}", err);
                self.synology = None;
            }
        }

        Ok(())
    }

    fn print_band_change(&mut self, now: DateTime<Tz>, band: &str, rx_bps: u64, tx_bps: u64) {
        if self.last_band.as_deref() == Some(band) {
            return;
        }

        println!(
            "{:<TIMESTAMP_FIELD_WIDTH$} band={:<BAND_FIELD_WIDTH$} tx={} rx={}",
            console_timestamp(now),
            band,
            format_bps(tx_bps),
            format_bps(rx_bps)
        );
        self.last_band = Some(band.to_string());
    }

    fn sleep_until_next_poll(&self) -> bool {
        let mut remaining = Duration::from_secs(POLL_INTERVAL_SECS);
        let sleep_slice = Duration::from_millis(SLEEP_SLICE_MILLIS);

        while remaining > Duration::ZERO {
            if self.received_signal.load(Ordering::Relaxed) != 0 {
                return true;
            }

            let current_sleep = remaining.min(sleep_slice);
            thread::sleep(current_sleep);
            remaining = remaining.saturating_sub(current_sleep);
        }

        self.received_signal.load(Ordering::Relaxed) != 0
    }
}

fn open_csv_file() -> Result<File> {
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

fn append_sample(file: &mut File, timestamp: &str, band: &str, rx_bps: u64, tx_bps: u64) -> Result<()> {
    writeln!(file, "{},{},{},{}", timestamp, band, rx_bps, tx_bps)?;
    file.flush()?;
    Ok(())
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

fn install_shutdown_handlers() -> Result<Arc<AtomicUsize>> {
    let received_signal = Arc::new(AtomicUsize::new(0));
    flag::register_usize(SIGINT, Arc::clone(&received_signal), SIGINT as usize)?;
    flag::register_usize(SIGTERM, Arc::clone(&received_signal), SIGTERM as usize)?;
    Ok(received_signal)
}

fn shutdown_signal_name(signal: usize) -> &'static str {
    match signal as i32 {
        SIGINT => "SIGINT",
        SIGTERM => "SIGTERM",
        _ => "UNKNOWN",
    }
}