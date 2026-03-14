mod monitor;
mod synology;
use anyhow::Result;
use serde::Deserialize;

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
    // `main` stays intentionally thin: load credentials, build the runtime, and hand off.
    println!("{} v{}", APP_NAME, APP_VERSION);
    let creds = read_credentials()?;
    let mut monitor = monitor::Monitor::new()?;
    monitor.run(&creds.username, &creds.password)
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error=fatal details={}", err);
        std::process::exit(1);
    }
}
