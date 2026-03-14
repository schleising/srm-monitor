use crate::synology::Synology;
use anyhow::Result;
use chrono::{DateTime, Datelike, Utc};
use chrono_tz::Tz;
use signal_hook::consts::signal::{SIGINT, SIGTERM};
use signal_hook::flag;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

const CSV_FILE_PATH: &str = "avg_rates.csv";
const NODE_ID: i32 = 8;
const POLL_INTERVAL_SECS: u64 = 30;
const TIMESTAMP_FIELD_WIDTH: usize = 25;
const BAND_FIELD_WIDTH: usize = 4;
const SLEEP_SLICE_MILLIS: u64 = 250;

pub(crate) trait RatesClient {
    fn fetch_avg_rates(&self, node_id: i32) -> Result<(String, u64, u64)>;
}

impl RatesClient for Synology {
    fn fetch_avg_rates(&self, node_id: i32) -> Result<(String, u64, u64)> {
        Synology::fetch_avg_rates(self, node_id)
    }
}

pub(crate) trait SessionConnector<S> {
    fn connect(&mut self, username: &str, password: &str) -> Result<S>;
}

pub(crate) struct LiveConnector;

impl SessionConnector<Synology> for LiveConnector {
    fn connect(&mut self, username: &str, password: &str) -> Result<Synology> {
        Synology::new(username, password)
    }
}

pub(crate) struct Monitor<S = Synology, C = LiveConnector, W = File> {
    timezone: Tz,
    received_signal: Arc<AtomicUsize>,
    csv_file: W,
    last_band: Option<String>,
    synology: Option<S>,
    connector: C,
}

impl Monitor<Synology, LiveConnector, File> {
    pub fn new() -> Result<Self> {
        Ok(Self::with_parts(
            local_timezone()?,
            install_shutdown_handlers()?,
            open_csv_file()?,
            LiveConnector,
        ))
    }
}

impl<S, C, W> Monitor<S, C, W>
where
    S: RatesClient,
    C: SessionConnector<S>,
    W: Write,
{
    fn with_parts(
        timezone: Tz,
        received_signal: Arc<AtomicUsize>,
        csv_file: W,
        connector: C,
    ) -> Self {
        Self {
            timezone,
            received_signal,
            csv_file,
            last_band: None,
            synology: None,
            connector,
        }
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

        match self.connector.connect(username, password) {
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
        if !should_emit_band_change(self.last_band.as_deref(), band) {
            return;
        }

        println!("{}", format_band_change_line(now, band, rx_bps, tx_bps));
        self.last_band = Some(band.to_string());
    }

    fn sleep_until_next_poll(&self) -> bool {
        sleep_until_signal_or_timeout(
            self.received_signal.as_ref(),
            Duration::from_secs(POLL_INTERVAL_SECS),
            Duration::from_millis(SLEEP_SLICE_MILLIS),
        )
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

fn should_emit_band_change(last_band: Option<&str>, band: &str) -> bool {
    last_band != Some(band)
}

fn format_band_change_line(now: DateTime<Tz>, band: &str, rx_bps: u64, tx_bps: u64) -> String {
    format!(
        "{:<TIMESTAMP_FIELD_WIDTH$} band={:<BAND_FIELD_WIDTH$} tx={} rx={}",
        console_timestamp(now),
        band,
        format_bps(tx_bps),
        format_bps(rx_bps)
    )
}

fn append_sample<W: Write>(
    file: &mut W,
    timestamp: &str,
    band: &str,
    rx_bps: u64,
    tx_bps: u64,
) -> Result<()> {
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

fn sleep_until_signal_or_timeout(
    received_signal: &AtomicUsize,
    poll_interval: Duration,
    sleep_slice: Duration,
) -> bool {
    let mut remaining = poll_interval;

    while remaining > Duration::ZERO {
        if received_signal.load(Ordering::Relaxed) != 0 {
            return true;
        }

        let current_sleep = remaining.min(sleep_slice);
        thread::sleep(current_sleep);
        remaining = remaining.saturating_sub(current_sleep);
    }

    received_signal.load(Ordering::Relaxed) != 0
}

fn shutdown_signal_name(signal: usize) -> &'static str {
    match signal as i32 {
        SIGINT => "SIGINT",
        SIGTERM => "SIGTERM",
        _ => "UNKNOWN",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    struct MockSession {
        fetch_results: RefCell<VecDeque<Result<(String, u64, u64)>>>,
    }

    impl MockSession {
        fn new(fetch_results: Vec<Result<(String, u64, u64)>>) -> Self {
            Self {
                fetch_results: RefCell::new(fetch_results.into()),
            }
        }
    }

    impl RatesClient for MockSession {
        fn fetch_avg_rates(&self, _node_id: i32) -> Result<(String, u64, u64)> {
            self.fetch_results
                .borrow_mut()
                .pop_front()
                .expect("expected queued fetch result")
        }
    }

    struct MockConnector {
        connect_results: VecDeque<Result<MockSession>>,
        calls: usize,
    }

    impl MockConnector {
        fn new(connect_results: Vec<Result<MockSession>>) -> Self {
            Self {
                connect_results: connect_results.into(),
                calls: 0,
            }
        }
    }

    impl SessionConnector<MockSession> for MockConnector {
        fn connect(&mut self, _username: &str, _password: &str) -> Result<MockSession> {
            self.calls += 1;
            self.connect_results
                .pop_front()
                .expect("expected queued connect result")
        }
    }

    fn test_monitor(connector: MockConnector) -> Monitor<MockSession, MockConnector, Vec<u8>> {
        Monitor::with_parts(
            chrono_tz::Europe::London,
            Arc::new(AtomicUsize::new(0)),
            Vec::new(),
            connector,
        )
    }

    #[test]
    fn emits_first_band_change() {
        assert!(should_emit_band_change(None, "5G-1"));
    }

    #[test]
    fn suppresses_same_band_change() {
        assert!(!should_emit_band_change(Some("5G-1"), "5G-1"));
    }

    #[test]
    fn emits_when_band_changes() {
        assert!(should_emit_band_change(Some("2.4G"), "5G-1"));
    }

    #[test]
    fn formats_band_change_line_with_fixed_columns() {
        let timezone = chrono_tz::Europe::London;
        let now = timezone.with_ymd_and_hms(2026, 3, 14, 21, 55, 0).unwrap();

        let line = format_band_change_line(now, "5G-1", 1_300_000_000, 1_404_000_000);

        assert_eq!(
            line,
            "Sat 14th Mar 2026 21:55 GMT band=5G-1 tx=1.404 Gbps rx=1.300 Gbps"
        );
    }

    #[test]
    fn console_timestamp_uses_bst_after_dst_change() {
        let timezone = chrono_tz::Europe::London;
        let now = timezone.with_ymd_and_hms(2026, 3, 29, 17, 35, 0).unwrap();

        assert_eq!(console_timestamp(now), "Sun 29th Mar 2026 17:35 BST");
    }

    #[test]
    fn day_suffix_handles_teens_and_ordinal_endings() {
        assert_eq!(day_suffix(11), "th");
        assert_eq!(day_suffix(12), "th");
        assert_eq!(day_suffix(13), "th");
        assert_eq!(day_suffix(21), "st");
        assert_eq!(day_suffix(22), "nd");
        assert_eq!(day_suffix(23), "rd");
        assert_eq!(day_suffix(24), "th");
    }

    #[test]
    fn format_bps_respects_unit_boundaries() {
        assert_eq!(format_bps(999), "999 bps");
        assert_eq!(format_bps(1_000), "1.000 Kbps");
        assert_eq!(format_bps(1_500_000), "1.500 Mbps");
    }

    #[test]
    fn shutdown_signal_name_maps_known_signals() {
        assert_eq!(shutdown_signal_name(SIGINT as usize), "SIGINT");
        assert_eq!(shutdown_signal_name(SIGTERM as usize), "SIGTERM");
        assert_eq!(shutdown_signal_name(999), "UNKNOWN");
    }

    #[test]
    fn sleep_returns_immediately_when_signal_is_already_set() {
        let signal = AtomicUsize::new(SIGTERM as usize);

        assert!(sleep_until_signal_or_timeout(
            &signal,
            Duration::from_secs(30),
            Duration::from_millis(250)
        ));
    }

    #[test]
    fn sleep_reports_no_signal_for_zero_duration() {
        let signal = AtomicUsize::new(0);

        assert!(!sleep_until_signal_or_timeout(
            &signal,
            Duration::ZERO,
            Duration::from_millis(250)
        ));
    }

    #[test]
    fn ensure_session_connects_once_when_missing() {
        let connector = MockConnector::new(vec![Ok(MockSession::new(vec![]))]);
        let mut monitor = test_monitor(connector);

        assert!(monitor.ensure_session("user", "pass"));
        assert_eq!(monitor.connector.calls, 1);
        assert!(monitor.synology.is_some());
    }

    #[test]
    fn ensure_session_does_not_reconnect_when_session_exists() {
        let connector = MockConnector::new(vec![Ok(MockSession::new(vec![]))]);
        let mut monitor = test_monitor(connector);

        assert!(monitor.ensure_session("user", "pass"));
        assert!(monitor.ensure_session("user", "pass"));

        assert_eq!(monitor.connector.calls, 1);
    }

    #[test]
    fn poll_once_writes_csv_and_updates_last_band() {
        let connector = MockConnector::new(vec![Ok(MockSession::new(vec![Ok((
            "5G-1".to_string(),
            1_300_000_000,
            1_404_000_000,
        ))]))]);
        let mut monitor = test_monitor(connector);

        assert!(monitor.ensure_session("user", "pass"));
        monitor.poll_once().unwrap();

        let csv = String::from_utf8(monitor.csv_file).unwrap();
        assert!(csv.contains(",5G-1,1300000000,1404000000\n"));
        assert_eq!(monitor.last_band.as_deref(), Some("5G-1"));
    }

    #[test]
    fn poll_once_clears_session_after_fetch_failure() {
        let connector = MockConnector::new(vec![Ok(MockSession::new(vec![Err(anyhow::anyhow!(
            "boom"
        ))]))]);
        let mut monitor = test_monitor(connector);

        assert!(monitor.ensure_session("user", "pass"));
        monitor.poll_once().unwrap();

        assert!(monitor.synology.is_none());
    }

    #[test]
    fn second_poll_with_changed_band_updates_last_band_and_appends_again() {
        let connector = MockConnector::new(vec![Ok(MockSession::new(vec![
            Ok(("5G-2".to_string(), 300, 400)),
            Ok(("5G-1".to_string(), 500, 600)),
        ]))]);
        let mut monitor = test_monitor(connector);

        assert!(monitor.ensure_session("user", "pass"));
        monitor.poll_once().unwrap();
        monitor.poll_once().unwrap();

        let csv = String::from_utf8(monitor.csv_file).unwrap();
        assert!(csv.contains(",5G-2,300,400\n"));
        assert!(csv.contains(",5G-1,500,600\n"));
        assert_eq!(monitor.last_band.as_deref(), Some("5G-1"));
    }
}
