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

    // Fetch avg rates (fetch_avg_rates handles login + fetch + logout)
    let (rx_gbps, tx_gbps) =
        synology::fetch_avg_rates(&creds.username, &creds.password, 8, "5G-1")?;

    println!("Avg TX Rate: {} Gbps", tx_gbps);
    println!("Avg RX Rate: {} Gbps", rx_gbps);

    // At this point you can store (rx_gbps, tx_gbps) in your DB
    Ok(())
}
