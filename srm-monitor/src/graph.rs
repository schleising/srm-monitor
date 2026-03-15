use anyhow::Result;
use chrono::{DateTime, Local, Utc};
use eframe::egui;
use egui::Vec2b;
use egui_plot::{GridMark, Legend, Line, Plot, PlotBounds, PlotPoints};
use srm_common::models::TelemetrySample;
use std::collections::VecDeque;
use std::ops::RangeInclusive;
use std::sync::{
    Arc, Mutex,
    mpsc::{Receiver, Sender},
};
use std::thread;

const PLOT_LINK_GROUP: &str = "telemetry-time";
const ROLLING_WINDOW_SECS: f64 = 5.0 * 60.0;
const THROUGHPUT_MIN_MBPS: f64 = 0.0;
const THROUGHPUT_MAX_MBPS: f64 = 2000.0;
const SIGNAL_MIN_PERCENT: f64 = 0.0;
const SIGNAL_MAX_PERCENT: f64 = 105.0;
const MIN_PLOT_HEIGHT: f32 = 160.0;
const PLOT_LABEL_OVERHEAD: f32 = 44.0;
const INTER_PLOT_SPACING: f32 = 12.0;
const MAX_RENDERED_POINTS: usize = 2048;

type PlotDatum = [f64; 2];
type PendingEvents = Arc<Mutex<VecDeque<GraphEvent>>>;

pub enum GraphEvent {
    ReplaceHistory(Vec<TelemetrySample>),
    Error(String),
}

pub enum GraphCommand {
    FollowLatest,
    LoadVisibleRange {
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    },
}

pub fn run_monitor_window(
    app_name: &str,
    app_version: &str,
    receiver: Receiver<GraphEvent>,
    command_sender: Sender<GraphCommand>,
    history_start: DateTime<Utc>,
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
        Box::new(move |creation_context| {
            Ok(Box::new(MonitorGraphApp::new(
                creation_context.egui_ctx.clone(),
                app_name,
                app_version,
                receiver,
                command_sender,
                history_start,
            )))
        }),
    )?;

    Ok(())
}

struct MonitorGraphApp {
    app_name: String,
    app_version: String,
    pending_events: PendingEvents,
    command_sender: Sender<GraphCommand>,
    latest_sample: Option<TelemetrySample>,
    rx_series: Vec<PlotDatum>,
    tx_series: Vec<PlotDatum>,
    signal_series: Vec<PlotDatum>,
    latest_error: Option<String>,
    follow_latest: bool,
    history_start: DateTime<Utc>,
    last_requested_range: Option<(f64, f64)>,
}

impl MonitorGraphApp {
    fn new(
        egui_context: egui::Context,
        app_name: String,
        app_version: String,
        receiver: Receiver<GraphEvent>,
        command_sender: Sender<GraphCommand>,
        history_start: DateTime<Utc>,
    ) -> Self {
        let pending_events = Arc::new(Mutex::new(VecDeque::new()));
        spawn_event_relay(receiver, pending_events.clone(), egui_context);

        Self {
            app_name,
            app_version,
            pending_events,
            command_sender,
            latest_sample: None,
            rx_series: Vec::new(),
            tx_series: Vec::new(),
            signal_series: Vec::new(),
            latest_error: None,
            follow_latest: true,
            history_start,
            last_requested_range: None,
        }
    }

    fn replace_history(&mut self, samples: Vec<TelemetrySample>) {
        self.rx_series.clear();
        self.tx_series.clear();
        self.signal_series.clear();

        for sample in samples {
            self.push_sample(sample);
        }
    }

    fn push_sample(&mut self, sample: TelemetrySample) {
        let timestamp = sample_timestamp(&sample);
        self.rx_series
            .push([timestamp, sample.rx_bps as f64 / 1_000_000.0]);
        self.tx_series
            .push([timestamp, sample.tx_bps as f64 / 1_000_000.0]);
        self.signal_series
            .push([timestamp, sample.signal_strength as f64]);
        if self
            .latest_sample
            .as_ref()
            .is_none_or(|current| sample.timestamp_utc >= current.timestamp_utc)
        {
            self.latest_sample = Some(sample);
        }
    }

    fn ingest_events(&mut self) -> bool {
        let Ok(mut pending_events) = self.pending_events.lock() else {
            self.latest_error = Some("UI event queue poisoned".to_string());
            return false;
        };
        let mut drained_events = VecDeque::new();
        std::mem::swap(&mut drained_events, &mut pending_events);
        drop(pending_events);

        let mut changed = false;
        while let Some(event) = drained_events.pop_front() {
            match event {
                GraphEvent::ReplaceHistory(samples) => {
                    self.replace_history(samples);
                    self.latest_error = None;
                    changed = true;
                }
                GraphEvent::Error(error) => {
                    self.latest_error = Some(error);
                    changed = true;
                }
            }
        }

        changed
    }

    fn latest_x_bounds(&self, current_bounds: PlotBounds) -> Option<(f64, f64)> {
        let first_ts = self.rx_series.first()?.first().copied()?;
        let latest_ts = self.rx_series.last()?.first().copied()?;
        let min_x = if latest_ts - first_ts > ROLLING_WINDOW_SECS {
            latest_ts - ROLLING_WINDOW_SECS
        } else {
            first_ts
        };
        let max_x = if latest_ts > min_x {
            latest_ts
        } else {
            min_x + current_bounds.width().max(1.0)
        };

        Some((min_x, max_x))
    }

    fn visible_points(&self, series: &[PlotDatum], min_x: f64, max_x: f64) -> PlotPoints<'static> {
        let visible = visible_range(series, min_x, max_x);
        let decimated = decimate_visible_points(visible, MAX_RENDERED_POINTS);
        PlotPoints::Owned(decimated)
    }

    fn render_plot_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let follow_response = ui.checkbox(&mut self.follow_latest, "Follow latest 5 minutes");
            if follow_response.changed() && self.follow_latest {
                self.last_requested_range = None;
                let _ = self.command_sender.send(GraphCommand::FollowLatest);
            }
            if ui.button("Jump to latest").clicked() {
                self.follow_latest = true;
                self.last_requested_range = None;
                let _ = self.command_sender.send(GraphCommand::FollowLatest);
            }
            ui.label(
                "Drag to pan and use the mouse wheel or trackpad to zoom out to older samples.",
            );
        });
    }

    fn build_plot<'a>(&self, id: &'a str, y_label: &'a str) -> Plot<'a> {
        Plot::new(id)
            .legend(Legend::default())
            .link_axis(PLOT_LINK_GROUP, [true, false])
            .allow_drag([true, false])
            .allow_scroll([true, false])
            .allow_zoom([true, false])
            .auto_bounds(Vec2b::new(false, false))
            .x_axis_formatter(format_time_axis)
            .label_formatter(move |name, value| format_plot_label(name, value.x, value.y, y_label))
    }

    fn apply_plot_bounds(&mut self, plot_ui: &mut egui_plot::PlotUi<'_>, min_y: f64, max_y: f64) {
        let current_bounds = plot_ui.plot_bounds();
        let (min_x, max_x) = if self.follow_latest {
            self.latest_x_bounds(current_bounds)
                .unwrap_or((current_bounds.min()[0], current_bounds.max()[0]))
        } else {
            (current_bounds.min()[0], current_bounds.max()[0])
        };

        plot_ui.set_plot_bounds(PlotBounds::from_min_max([min_x, min_y], [max_x, max_y]));
        plot_ui.set_auto_bounds(Vec2b::new(false, false));
    }

    fn sync_follow_mode(&mut self, ctx: &egui::Context, response: &egui::Response) {
        if response.dragged() {
            self.follow_latest = false;
        }
        let x_scrolled = ctx.input(|input| {
            response.hovered()
                && (input.raw_scroll_delta.x != 0.0 || input.raw_scroll_delta.y != 0.0)
        });
        let zoomed = ctx
            .input(|input| response.hovered() && (input.zoom_delta() - 1.0).abs() > f32::EPSILON);
        if x_scrolled || zoomed {
            self.follow_latest = false;
            self.last_requested_range = None;
        }
    }

    fn maybe_request_visible_range(&mut self, min_x: f64, max_x: f64) {
        if self.follow_latest {
            return;
        }

        let history_start_x = self.history_start.timestamp_millis() as f64 / 1000.0;
        let requested_min_x = min_x.max(history_start_x);
        let requested_max_x = max_x.max(requested_min_x + 1.0);

        if self
            .last_requested_range
            .is_some_and(|(last_min, last_max)| {
                (requested_min_x - last_min).abs() < 1.0 && (requested_max_x - last_max).abs() < 1.0
            })
        {
            return;
        }

        let Some(start) =
            DateTime::from_timestamp_millis((requested_min_x * 1000.0).floor() as i64)
        else {
            return;
        };
        let Some(end) = DateTime::from_timestamp_millis((requested_max_x * 1000.0).ceil() as i64)
        else {
            return;
        };

        if self
            .command_sender
            .send(GraphCommand::LoadVisibleRange { start, end })
            .is_ok()
        {
            self.last_requested_range = Some((requested_min_x, requested_max_x));
        }
    }
}

impl eframe::App for MonitorGraphApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let _ = self.ingest_events();

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading(format!("{} {}", self.app_name, self.app_version));
            ui.label("Live SRM telemetry retrieved from the HTTP API.");
            ui.separator();

            if let Some(sample) = self.latest_sample.as_ref() {
                ui.horizontal_wrapped(|ui| {
                    ui.strong(format!("Current band: {}", sample.band));
                    ui.label(format!("Signal: {}%", sample.signal_strength));
                    ui.label(format!(
                        "Rx: {:.3} Mbps",
                        sample.rx_bps as f64 / 1_000_000.0
                    ));
                    ui.label(format!(
                        "Tx: {:.3} Mbps",
                        sample.tx_bps as f64 / 1_000_000.0
                    ));
                    ui.label(format!("Updated: {}", local_timestamp(sample)));
                });
            } else {
                ui.label("Waiting for telemetry from the API...");
            }

            if let Some(error) = &self.latest_error {
                ui.colored_label(egui::Color32::from_rgb(196, 51, 51), error);
            }

            ui.add_space(8.0);
            self.render_plot_controls(ui);
            ui.add_space(6.0);

            let remaining_height = ui.available_height();
            let plot_height = ((remaining_height - INTER_PLOT_SPACING - PLOT_LABEL_OVERHEAD) / 2.0)
                .max(MIN_PLOT_HEIGHT);

            ui.label("Throughput history in Mbps. The x-axis shows local wall clock time.");
            let throughput_plot = self
                .build_plot("throughput-plot", "Mbps")
                .height(plot_height)
                .show(ui, |plot_ui| {
                    self.apply_plot_bounds(plot_ui, THROUGHPUT_MIN_MBPS, THROUGHPUT_MAX_MBPS);
                    let bounds = plot_ui.plot_bounds();
                    plot_ui.line(
                        Line::new(
                            "Rx",
                            self.visible_points(
                                &self.rx_series,
                                bounds.min()[0],
                                bounds.max()[0],
                            ),
                        )
                        .color(egui::Color32::from_rgb(34, 139, 230)),
                    );
                    plot_ui.line(
                        Line::new(
                            "Tx",
                            self.visible_points(
                                &self.tx_series,
                                bounds.min()[0],
                                bounds.max()[0],
                            ),
                        )
                        .color(egui::Color32::from_rgb(231, 111, 81)),
                    );
                });
            self.maybe_request_visible_range(
                throughput_plot.transform.bounds().min()[0],
                throughput_plot.transform.bounds().max()[0],
            );
            self.sync_follow_mode(ctx, &throughput_plot.response);

            ui.add_space(INTER_PLOT_SPACING);
            ui.label("Signal strength as a percentage.");
            let signal_plot = self
                .build_plot("signal-plot", "%")
                .height(plot_height)
                .show(ui, |plot_ui| {
                    self.apply_plot_bounds(plot_ui, SIGNAL_MIN_PERCENT, SIGNAL_MAX_PERCENT);
                    let bounds = plot_ui.plot_bounds();
                    plot_ui.line(
                        Line::new(
                            "Signal",
                            self.visible_points(
                                &self.signal_series,
                                bounds.min()[0],
                                bounds.max()[0],
                            ),
                        )
                        .color(egui::Color32::from_rgb(46, 196, 182)),
                    );
                });
            self.sync_follow_mode(ctx, &signal_plot.response);
        });
    }
}

fn spawn_event_relay(
    receiver: Receiver<GraphEvent>,
    pending_events: PendingEvents,
    ctx: egui::Context,
) {
    let _ = thread::Builder::new()
        .name("srm-gui-events".to_string())
        .spawn(move || {
            while let Ok(event) = receiver.recv() {
                if let Ok(mut queue) = pending_events.lock() {
                    queue.push_back(event);
                } else {
                    break;
                }
                ctx.request_repaint();
            }
            ctx.request_repaint();
        });
}

fn visible_range(series: &[PlotDatum], min_x: f64, max_x: f64) -> &[PlotDatum] {
    if series.is_empty() {
        return series;
    }

    let start = series
        .partition_point(|point| point[0] < min_x)
        .saturating_sub(1);
    let end = (series.partition_point(|point| point[0] <= max_x) + 1).min(series.len());

    if start >= end {
        &series[series.len() - 1..]
    } else {
        &series[start..end]
    }
}

fn decimate_visible_points(points: &[PlotDatum], max_points: usize) -> Vec<egui_plot::PlotPoint> {
    if points.is_empty() || max_points == 0 {
        return Vec::new();
    }

    if max_points == 1 {
        return vec![points[points.len() - 1].into()];
    }

    if points.len() <= max_points {
        return points.iter().copied().map(Into::into).collect();
    }

    let stride = ((points.len() - 1) / (max_points - 1)).max(1);
    let mut reduced = Vec::with_capacity(max_points);
    let mut index = 0usize;
    while index < points.len() - 1 && reduced.len() < max_points - 1 {
        reduced.push(points[index].into());
        index += stride;
    }

    let last_point: egui_plot::PlotPoint = points[points.len() - 1].into();
    if reduced.last().copied() != Some(last_point) {
        reduced.push(last_point);
    }

    reduced
}

fn sample_timestamp(sample: &TelemetrySample) -> f64 {
    sample.timestamp_utc.timestamp_millis() as f64 / 1000.0
}

fn local_timestamp(sample: &TelemetrySample) -> String {
    let local_time: DateTime<Local> = sample.timestamp_utc.with_timezone(&Local);
    local_time.format("%Y-%m-%d %H:%M:%S %Z").to_string()
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
    use chrono::Utc;

    #[test]
    fn formats_timestamp_for_axis_labels() {
        let utc_time = DateTime::parse_from_rfc3339("2026-03-15T18:44:12+00:00")
            .unwrap()
            .with_timezone(&Utc);
        let expected = utc_time
            .with_timezone(&Local)
            .format("%H:%M:%S")
            .to_string();

        assert_eq!(
            format_timestamp(sample_timestamp(&TelemetrySample::new(
                utc_time,
                "5G-1".to_string(),
                54,
                1_200_000,
                2_400_000,
            ))),
            expected
        );
    }

    #[test]
    fn visible_range_includes_boundary_points() {
        let series = vec![[0.0, 1.0], [10.0, 2.0], [20.0, 3.0], [30.0, 4.0]];

        let visible = visible_range(&series, 12.0, 22.0);

        assert_eq!(visible, &[[10.0, 2.0], [20.0, 3.0], [30.0, 4.0]]);
    }

    #[test]
    fn decimation_caps_rendered_points() {
        let points: Vec<_> = (0..10_000)
            .map(|index| [index as f64, index as f64 / 10.0])
            .collect();

        let reduced = decimate_visible_points(&points, 512);

        assert!(reduced.len() <= 512);
        assert_eq!(reduced.first().unwrap().x, 0.0);
        assert_eq!(reduced.last().unwrap().x, 9_999.0);
    }

    #[test]
    fn replace_history_keeps_newest_known_sample() {
        let (_event_sender, event_receiver) = std::sync::mpsc::channel();
        let (command_sender, _command_receiver) = std::sync::mpsc::channel();
        let history_start = DateTime::parse_from_rfc3339("2026-03-15T18:30:00+00:00")
            .unwrap()
            .with_timezone(&Utc);
        let mut app = MonitorGraphApp::new(
            egui::Context::default(),
            "test".to_string(),
            "0.0.0".to_string(),
            event_receiver,
            command_sender,
            history_start,
        );

        let latest = TelemetrySample::new(
            DateTime::parse_from_rfc3339("2026-03-15T18:35:00+00:00")
                .unwrap()
                .with_timezone(&Utc),
            "5G-1".to_string(),
            70,
            100,
            200,
        );
        app.push_sample(latest.clone());

        app.replace_history(vec![TelemetrySample::new(
            DateTime::parse_from_rfc3339("2026-03-15T18:34:00+00:00")
                .unwrap()
                .with_timezone(&Utc),
            "5G-1".to_string(),
            69,
            90,
            180,
        )]);

        assert_eq!(
            app.latest_sample.as_ref().unwrap().timestamp_utc,
            latest.timestamp_utc
        );
    }

    #[test]
    fn visible_range_requests_are_debounced() {
        let (_event_sender, event_receiver) = std::sync::mpsc::channel();
        let (command_sender, command_receiver) = std::sync::mpsc::channel();
        let history_start = DateTime::parse_from_rfc3339("2026-03-15T18:30:00+00:00")
            .unwrap()
            .with_timezone(&Utc);
        let mut app = MonitorGraphApp::new(
            egui::Context::default(),
            "test".to_string(),
            "0.0.0".to_string(),
            event_receiver,
            command_sender,
            history_start,
        );

        app.follow_latest = false;
        app.maybe_request_visible_range(100.0, 200.0);
        app.maybe_request_visible_range(100.4, 200.4);

        assert!(matches!(
            command_receiver.recv().unwrap(),
            GraphCommand::LoadVisibleRange { .. }
        ));
        assert!(command_receiver.try_recv().is_err());
    }
}
