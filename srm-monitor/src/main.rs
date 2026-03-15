mod graph;
mod monitor;
mod profiling;
mod synology;
use anyhow::{Result, anyhow};
use serde::Deserialize;
use std::thread;

const APP_NAME: &str = env!("CARGO_PKG_NAME");
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

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

fn run() -> Result<()> {
    let _profiling_session = profiling::init_from_env()?;
    println!("{} v{}", APP_NAME, APP_VERSION);
    let creds = read_credentials()?;
    let shutdown_signal = monitor::install_shutdown_handlers()?;
    let (event_sender, event_receiver) = std::sync::mpsc::channel();

    let username = creds.username;
    let password = creds.password;
    let monitor_shutdown_signal = shutdown_signal.clone();
    let monitor_handle = thread::Builder::new()
        .name("srm-monitor-poller".to_string())
        .spawn(move || -> Result<()> {
            let mut monitor = monitor::Monitor::new(monitor_shutdown_signal, Some(event_sender))?;
            monitor.run(&username, &password)
        })?;

    let graph_result = graph::run_monitor_window(
        APP_NAME,
        APP_VERSION,
        event_receiver,
        shutdown_signal.clone(),
    );

    monitor::request_application_shutdown(shutdown_signal.as_ref());
    let monitor_result = match monitor_handle.join() {
        Ok(result) => result,
        Err(_) => Err(anyhow!("monitor thread panicked")),
    };

    graph_result?;
    monitor_result
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error=fatal details={}", err);
        std::process::exit(1);
    }
}
