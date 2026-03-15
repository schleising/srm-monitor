use crate::monitor::{MonitorEvent, MonitorSample, request_application_shutdown};
use anyhow::Result;
use chrono::{DateTime, Local};
use eframe::egui;
use egui_plot::{Legend, Line, Plot, PlotPoints};
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::Receiver;
use std::time::Duration;

const HISTORY_LIMIT: usize = 240;
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
}

impl MonitorGraphApp {
    fn new(
        app_name: String,
        app_version: String,
        receiver: Receiver<MonitorEvent>,
        shutdown_signal: Arc<AtomicUsize>,
    ) -> Self {
        Self {
            app_name,
            app_version,
            receiver,
            shutdown_signal,
            samples: VecDeque::with_capacity(HISTORY_LIMIT),
            latest_error: None,
        }
    }

    fn ingest_events(&mut self) {
        for event in self.receiver.try_iter() {
            match event {
                MonitorEvent::Sample(sample) => {
                    if self.samples.len() == HISTORY_LIMIT {
                        self.samples.pop_front();
                    }
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
        PlotPoints::from_iter(self.samples.iter().enumerate().map(|(index, sample)| {
            [index as f64, selector(sample) as f64 / 1_000_000.0]
        }))
    }

    fn signal_points(&self) -> PlotPoints<'static> {
        PlotPoints::from_iter(
            self.samples
                .iter()
                .enumerate()
                .map(|(index, sample)| [index as f64, sample.signal_strength as f64]),
        )
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
            ui.label("Throughput history in Mbps. X-axis is the sample sequence at the 1 second polling interval.");
            Plot::new("throughput-plot")
                .legend(Legend::default())
                .height(280.0)
                .show(ui, |plot_ui| {
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

            ui.add_space(12.0);
            ui.label("Signal strength in dBm.");
            Plot::new("signal-plot")
                .legend(Legend::default())
                .height(220.0)
                .show(ui, |plot_ui| {
                    plot_ui.line(
                        Line::new(self.signal_points())
                            .name("Signal")
                            .color(egui::Color32::from_rgb(46, 196, 182)),
                    );
                });
        });
    }
}