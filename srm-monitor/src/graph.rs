use crate::monitor::{MonitorEvent, MonitorSample, request_application_shutdown};
use anyhow::{Result, anyhow};
use chrono::{DateTime, Local, Utc};
use eframe::egui;
use egui::Vec2b;
use egui_plot::{GridMark, Legend, Line, Plot, PlotBounds, PlotPoints};
use std::collections::VecDeque;
use std::fs;
use std::ops::RangeInclusive;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::Receiver;
use std::time::Duration;

const CSV_FILE_PATH: &str = "avg_rates.csv";
const PLOT_LINK_GROUP: &str = "telemetry-time";
const ROLLING_WINDOW_SECS: f64 = 5.0 * 60.0;
const REPAINT_INTERVAL: Duration = Duration::from_millis(250);

pub fn run_monitor_window(
    app_name: &str,
    app_version: &str,
    receiver: Receiver<MonitorEvent>,
    shutdown_signal: Arc<AtomicUsize>,
) -> Result<()> {
    let window_title = format!("{} {}", app_name, app_version);
    let app_name = app_name.to_string();
    let app_version = app_version.to_string();
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_title(window_title.clone())
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([960.0, 640.0]),
        ..Default::default()
    };

    eframe::run_native(
        &window_title,
        options,
        Box::new(move |_creation_context| {
            Ok(Box::new(MonitorGraphApp::new(
                app_name,
                app_version,
                receiver,
                shutdown_signal,
            )))
        }),
    )?;

    Ok(())
}

struct MonitorGraphApp {
    app_name: String,
    app_version: String,
    receiver: Receiver<MonitorEvent>,
    shutdown_signal: Arc<AtomicUsize>,
    samples: VecDeque<MonitorSample>,
    latest_error: Option<String>,
    follow_latest: bool,
}

impl MonitorGraphApp {
    fn new(
        app_name: String,
        app_version: String,
        receiver: Receiver<MonitorEvent>,
        shutdown_signal: Arc<AtomicUsize>,
    ) -> Self {
        let (samples, latest_error) = match load_history_samples() {
            Ok(samples) => (samples, None),
            Err(error) => (VecDeque::new(), Some(format!("History load failed: {}", error))),
        };

        Self {
            app_name,
            app_version,
            receiver,
            shutdown_signal,
            samples,
            latest_error,
            follow_latest: true,
        }
    }

    fn ingest_events(&mut self) {
        for event in self.receiver.try_iter() {
            match event {
                MonitorEvent::Sample(sample) => {
                    self.samples.push_back(sample);
                    self.latest_error = None;
                }
                MonitorEvent::Error(error) => {
                    self.latest_error = Some(error);
                }
            }
        }
    }

    fn latest_sample(&self) -> Option<&MonitorSample> {
        self.samples.back()
    }

    fn local_timestamp(sample: &MonitorSample) -> String {
        let local_time: DateTime<Local> = sample.captured_at.with_timezone(&Local);
        local_time.format("%Y-%m-%d %H:%M:%S %Z").to_string()
    }

    fn throughput_points(&self, selector: impl Fn(&MonitorSample) -> u64) -> PlotPoints<'static> {
        PlotPoints::from_iter(self.samples.iter().map(|sample| {
            [sample_timestamp(sample), selector(sample) as f64 / 1_000_000.0]
        }))
    }

    fn signal_points(&self) -> PlotPoints<'static> {
        PlotPoints::from_iter(self.samples.iter().map(|sample| {
            [sample_timestamp(sample), sample.signal_strength as f64]
        }))
    }

    fn time_bounds(&self, current_bounds: PlotBounds) -> Option<PlotBounds> {
        let first = self.samples.front()?;
        let latest = self.samples.back()?;

        let latest_ts = sample_timestamp(latest);
        let first_ts = sample_timestamp(first);
        let min_x = if latest_ts - first_ts > ROLLING_WINDOW_SECS {
            latest_ts - ROLLING_WINDOW_SECS
        } else {
            first_ts
        };

        Some(PlotBounds::from_min_max(
            [min_x, current_bounds.min()[1]],
            [latest_ts, current_bounds.max()[1]],
        ))
    }

    fn render_plot_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.follow_latest, "Follow latest 5 minutes");
            if ui.button("Jump to latest").clicked() {
                self.follow_latest = true;
            }
            ui.label("Drag to pan and use the mouse wheel or trackpad to zoom out to older samples.");
        });
    }

    fn build_plot<'a>(&self, id: &'a str, y_label: &'a str) -> Plot<'a> {
        Plot::new(id)
            .legend(Legend::default())
            .link_axis(PLOT_LINK_GROUP, [true, false])
            .allow_drag([true, true])
            .allow_scroll([true, true])
            .allow_zoom([true, true])
            .auto_bounds(Vec2b::new(false, true))
            .x_axis_formatter(format_time_axis)
            .label_formatter(move |name, value| format_plot_label(name, value.x, value.y, y_label))
    }

    fn maybe_follow_latest(&mut self, plot_ui: &mut egui_plot::PlotUi<'_>) {
        if !self.follow_latest {
            return;
        }

        if let Some(bounds) = self.time_bounds(plot_ui.plot_bounds()) {
            plot_ui.set_plot_bounds(bounds);
            plot_ui.set_auto_bounds(Vec2b::new(false, true));
        }
    }

    fn sync_follow_mode(&mut self, response: &egui::Response) {
        if response.dragged() {
            self.follow_latest = false;
        }
        if response.clicked() {
            self.follow_latest = false;
        }
    }
}

impl Drop for MonitorGraphApp {
    fn drop(&mut self) {
        request_application_shutdown(self.shutdown_signal.as_ref());
    }
}

impl eframe::App for MonitorGraphApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.ingest_events();
        ctx.request_repaint_after(REPAINT_INTERVAL);

        if self.shutdown_signal.load(Ordering::Relaxed) != 0 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading(format!("{} {}", self.app_name, self.app_version));
            ui.label("Live SRM uplink telemetry rendered with the native wgpu backend.");
            ui.separator();

            if let Some(sample) = self.latest_sample() {
                ui.horizontal_wrapped(|ui| {
                    ui.strong(format!("Current band: {}", sample.band));
                    ui.label(format!("Signal: {} dBm", sample.signal_strength));
                    ui.label(format!("Rx: {:.3} Mbps", sample.rx_bps as f64 / 1_000_000.0));
                    ui.label(format!("Tx: {:.3} Mbps", sample.tx_bps as f64 / 1_000_000.0));
                    ui.label(format!("Updated: {}", Self::local_timestamp(sample)));
                });
            } else {
                ui.label("Waiting for the first SRM sample...");
            }

            if let Some(error) = &self.latest_error {
                ui.colored_label(egui::Color32::from_rgb(196, 51, 51), error);
            }

            ui.add_space(8.0);
            self.render_plot_controls(ui);
            ui.add_space(6.0);
            ui.label("Throughput history in Mbps. The x-axis shows local wall clock time.");
            let throughput_plot = self
                .build_plot("throughput-plot", "Mbps")
                .height(280.0)
                .show(ui, |plot_ui| {
                    self.maybe_follow_latest(plot_ui);
                    plot_ui.line(
                        Line::new(self.throughput_points(|sample| sample.rx_bps))
                            .name("Rx")
                            .color(egui::Color32::from_rgb(34, 139, 230)),
                    );
                    plot_ui.line(
                        Line::new(self.throughput_points(|sample| sample.tx_bps))
                            .name("Tx")
                            .color(egui::Color32::from_rgb(231, 111, 81)),
                    );
                });
            self.sync_follow_mode(&throughput_plot.response);

            ui.add_space(12.0);
            ui.label("Signal strength in dBm.");
            let signal_plot = self
                .build_plot("signal-plot", "dBm")
                .height(220.0)
                .show(ui, |plot_ui| {
                    self.maybe_follow_latest(plot_ui);
                    plot_ui.line(
                        Line::new(self.signal_points())
                            .name("Signal")
                            .color(egui::Color32::from_rgb(46, 196, 182)),
                    );
                });
            self.sync_follow_mode(&signal_plot.response);
        });
    }
}

fn load_history_samples() -> Result<VecDeque<MonitorSample>> {
    let contents = match fs::read_to_string(CSV_FILE_PATH) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(VecDeque::new()),
        Err(error) => return Err(error.into()),
    };

    let mut samples = VecDeque::new();
    for (index, line) in contents.lines().enumerate() {
        if index == 0 || line.trim().is_empty() {
            continue;
        }
        samples.push_back(parse_history_line(line)?);
    }

    Ok(samples)
}

fn parse_history_line(line: &str) -> Result<MonitorSample> {
    let mut fields = line.split(',');
    let timestamp = fields
        .next()
        .ok_or_else(|| anyhow!("missing timestamp field"))?;
    let band = fields
        .next()
        .ok_or_else(|| anyhow!("missing band field"))?
        .to_string();
    let signal_strength = fields
        .next()
        .ok_or_else(|| anyhow!("missing signal strength field"))?
        .parse()?;
    let rx_bps = fields
        .next()
        .ok_or_else(|| anyhow!("missing rx field"))?
        .parse()?;
    let tx_bps = fields
        .next()
        .ok_or_else(|| anyhow!("missing tx field"))?
        .parse()?;

    if fields.next().is_some() {
        return Err(anyhow!("unexpected extra CSV fields"));
    }

    let captured_at = DateTime::parse_from_rfc3339(timestamp)?.with_timezone(&Utc);
    Ok(MonitorSample {
        captured_at,
        band,
        signal_strength,
        rx_bps,
        tx_bps,
    })
}

fn sample_timestamp(sample: &MonitorSample) -> f64 {
    sample.captured_at.timestamp_millis() as f64 / 1000.0
}

fn format_time_axis(mark: GridMark, _range: &RangeInclusive<f64>) -> String {
    format_timestamp(mark.value)
}

fn format_timestamp(timestamp_secs: f64) -> String {
    let timestamp_millis = (timestamp_secs * 1000.0).round() as i64;
    let Some(utc_time) = DateTime::from_timestamp_millis(timestamp_millis) else {
        return String::new();
    };

    utc_time
        .with_timezone(&Local)
        .format("%H:%M:%S")
        .to_string()
}

fn format_plot_label(name: &str, x: f64, y: f64, y_label: &str) -> String {
    format!("{}\n{}\n{:.3} {}", name, format_timestamp(x), y, y_label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_history_line() {
        let sample = parse_history_line("2026-03-15T18:44:12+00:00,5G-1,-54,1200000,2400000").unwrap();

        assert_eq!(sample.band, "5G-1");
        assert_eq!(sample.signal_strength, -54);
        assert_eq!(sample.rx_bps, 1_200_000);
        assert_eq!(sample.tx_bps, 2_400_000);
    }

    #[test]
    fn rejects_history_line_with_extra_fields() {
        let error = parse_history_line("2026-03-15T18:44:12+00:00,5G-1,-54,1200000,2400000,extra").unwrap_err();

        assert!(error.to_string().contains("unexpected extra CSV fields"));
    }

    #[test]
    fn formats_timestamp_for_axis_labels() {
        let utc_time = DateTime::parse_from_rfc3339("2026-03-15T18:44:12+00:00")
            .unwrap()
            .with_timezone(&Utc);
        let expected = utc_time.with_timezone(&Local).format("%H:%M:%S").to_string();

        assert_eq!(format_timestamp(sample_timestamp(&MonitorSample {
            captured_at: utc_time,
            band: "5G-1".to_string(),
            signal_strength: -54,
            rx_bps: 1_200_000,
            tx_bps: 2_400_000,
        })), expected);
    }
}