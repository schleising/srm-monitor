mod synology;
use serde::Deserialize;
use std::error::Error;

#[derive(Deserialize)]
struct TomlCredentials {
    credentials: Credentials,
}

#[derive(Deserialize)]
struct Credentials {
    username: String,
    password: String,
}

fn read_credentials() -> Result<Credentials, Box<dyn Error>> {
    let s = std::fs::read_to_string("secrets/srm_login.toml")?;
    let cfg: TomlCredentials = toml::from_str(&s)?;
    Ok(cfg.credentials)
}

fn main() -> Result<(), Box<dyn Error>> {
    let creds = read_credentials()?;

    let synology = synology::Synology::new(&creds.username, &creds.password)?;
    let (rx_gbps, tx_gbps) = synology.fetch_avg_rates(8, "5G-1")?;

    println!("Avg TX Rate: {} Gbps", tx_gbps);
    println!("Avg RX Rate: {} Gbps", rx_gbps);

    // At this point you can store (rx_gbps, tx_gbps) in your DB
    Ok(())
}
