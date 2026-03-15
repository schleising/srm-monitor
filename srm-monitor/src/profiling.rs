use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

const PROFILE_ENV_VAR: &str = "SRM_PROFILE";
const OUTPUT_DIR: &str = "instrumentation/latest";

static PROFILER: OnceLock<Arc<Profiler>> = OnceLock::new();

pub struct ProfilingSession {
    enabled: bool,
}

impl Drop for ProfilingSession {
    fn drop(&mut self) {
        if self.enabled {
            shutdown();
        }
    }
}

pub struct ProfileScope {
    name: &'static str,
    started_at: Instant,
}

impl Drop for ProfileScope {
    fn drop(&mut self) {
        if let Some(profiler) = PROFILER.get() {
            profiler.record_span(self.name, self.started_at.elapsed());
        }
    }
}

pub fn init_from_env() -> Result<ProfilingSession> {
    if !profiling_enabled() {
        return Ok(ProfilingSession { enabled: false });
    }

    let output_dir = PathBuf::from(OUTPUT_DIR);
    fs::create_dir_all(&output_dir)?;

    let profiler = Arc::new(Profiler::new(output_dir)?);
    let _ = PROFILER.set(profiler);

    Ok(ProfilingSession { enabled: true })
}

pub fn scope(name: &'static str) -> ProfileScope {
    ProfileScope {
        name,
        started_at: Instant::now(),
    }
}

pub fn record_metric(name: &str, value: f64, unit: &'static str) {
    if let Some(profiler) = PROFILER.get() {
        profiler.record_metric(name, value, unit);
    }
}

pub fn shutdown() {
    if let Some(profiler) = PROFILER.get() {
        let _ = profiler.write_summary();
    }
}

fn profiling_enabled() -> bool {
    std::env::var(PROFILE_ENV_VAR)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

struct Profiler {
    started_at: DateTime<Utc>,
    trace_path: PathBuf,
    summary_path: PathBuf,
    writer: Mutex<BufWriter<File>>,
    span_stats: Mutex<BTreeMap<&'static str, SpanStats>>,
    metric_stats: Mutex<BTreeMap<String, MetricStats>>,
    summary_written: AtomicBool,
}

impl Profiler {
    fn new(output_dir: PathBuf) -> Result<Self> {
        let trace_path = output_dir.join("trace.ndjson");
        let summary_path = output_dir.join("summary.json");
        let writer = BufWriter::new(File::create(&trace_path)?);

        Ok(Self {
            started_at: Utc::now(),
            trace_path,
            summary_path,
            writer: Mutex::new(writer),
            span_stats: Mutex::new(BTreeMap::new()),
            metric_stats: Mutex::new(BTreeMap::new()),
            summary_written: AtomicBool::new(false),
        })
    }

    fn record_span(&self, name: &'static str, duration: std::time::Duration) {
        let duration_ns = duration.as_nanos() as u64;

        if let Ok(mut stats) = self.span_stats.lock() {
            let entry = stats.entry(name).or_default();
            entry.count += 1;
            entry.total_ns += duration_ns as u128;
            entry.max_ns = entry.max_ns.max(duration_ns);
        }

        let _ = self.write_trace_event(&TraceEvent::span(name, duration_ns));
    }

    fn record_metric(&self, name: &str, value: f64, unit: &'static str) {
        if let Ok(mut stats) = self.metric_stats.lock() {
            let entry = stats
                .entry(name.to_string())
                .or_insert_with(|| MetricStats::new(unit));
            entry.observe(value);
        }

        let _ = self.write_trace_event(&TraceEvent::metric(name, value, unit));
    }

    fn write_trace_event(&self, event: &TraceEvent<'_>) -> Result<()> {
        let Ok(mut writer) = self.writer.lock() else {
            return Ok(());
        };

        serde_json::to_writer(&mut *writer, event)?;
        writer.write_all(b"\n")?;
        Ok(())
    }

    fn write_summary(&self) -> Result<()> {
        if self.summary_written.swap(true, Ordering::Relaxed) {
            return Ok(());
        }

        if let Ok(mut writer) = self.writer.lock() {
            writer.flush()?;
        }

        let mut spans = self
            .span_stats
            .lock()
            .map(|stats| {
                stats
                    .iter()
                    .map(|(name, stat)| SpanSummary::from_stats(name, stat))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        spans.sort_by(|left, right| right.total_ms.total_cmp(&left.total_ms));

        let mut metrics = self
            .metric_stats
            .lock()
            .map(|stats| {
                stats
                    .iter()
                    .map(|(name, stat)| MetricSummary::from_stats(name, stat))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        metrics.sort_by(|left, right| left.name.cmp(&right.name));

        let summary = SummaryFile {
            profile_env_var: PROFILE_ENV_VAR,
            started_at_utc: self.started_at.to_rfc3339(),
            finished_at_utc: Utc::now().to_rfc3339(),
            trace_file: self.trace_path.to_string_lossy().replace('\\', "/"),
            span_summary: spans,
            metric_summary: metrics,
        };

        let summary_file = File::create(&self.summary_path)?;
        serde_json::to_writer_pretty(summary_file, &summary)?;
        Ok(())
    }
}

#[derive(Default)]
struct SpanStats {
    count: u64,
    total_ns: u128,
    max_ns: u64,
}

struct MetricStats {
    count: u64,
    total: f64,
    min: f64,
    max: f64,
    last: f64,
    unit: &'static str,
}

impl MetricStats {
    fn new(unit: &'static str) -> Self {
        Self {
            count: 0,
            total: 0.0,
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
            last: 0.0,
            unit,
        }
    }

    fn observe(&mut self, value: f64) {
        self.count += 1;
        self.total += value;
        self.min = self.min.min(value);
        self.max = self.max.max(value);
        self.last = value;
    }
}

#[derive(Serialize)]
struct TraceEvent<'a> {
    kind: &'a str,
    name: &'a str,
    timestamp_utc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    unit: Option<&'a str>,
}

impl<'a> TraceEvent<'a> {
    fn span(name: &'a str, duration_ns: u64) -> Self {
        Self {
            kind: "span",
            name,
            timestamp_utc: Utc::now().to_rfc3339(),
            duration_ms: Some(duration_ns as f64 / 1_000_000.0),
            value: None,
            unit: None,
        }
    }

    fn metric(name: &'a str, value: f64, unit: &'a str) -> Self {
        Self {
            kind: "metric",
            name,
            timestamp_utc: Utc::now().to_rfc3339(),
            duration_ms: None,
            value: Some(value),
            unit: Some(unit),
        }
    }
}

#[derive(Serialize)]
struct SummaryFile {
    profile_env_var: &'static str,
    started_at_utc: String,
    finished_at_utc: String,
    trace_file: String,
    span_summary: Vec<SpanSummary>,
    metric_summary: Vec<MetricSummary>,
}

#[derive(Serialize)]
struct SpanSummary {
    name: String,
    count: u64,
    total_ms: f64,
    avg_ms: f64,
    max_ms: f64,
}

impl SpanSummary {
    fn from_stats(name: &str, stats: &SpanStats) -> Self {
        let total_ms = stats.total_ns as f64 / 1_000_000.0;
        let avg_ms = if stats.count == 0 {
            0.0
        } else {
            total_ms / stats.count as f64
        };

        Self {
            name: name.to_string(),
            count: stats.count,
            total_ms,
            avg_ms,
            max_ms: stats.max_ns as f64 / 1_000_000.0,
        }
    }
}

#[derive(Serialize)]
struct MetricSummary {
    name: String,
    count: u64,
    min: f64,
    max: f64,
    avg: f64,
    last: f64,
    unit: String,
}

impl MetricSummary {
    fn from_stats(name: &str, stats: &MetricStats) -> Self {
        let avg = if stats.count == 0 {
            0.0
        } else {
            stats.total / stats.count as f64
        };

        Self {
            name: name.to_string(),
            count: stats.count,
            min: stats.min,
            max: stats.max,
            avg,
            last: stats.last,
            unit: stats.unit.to_string(),
        }
    }
}
