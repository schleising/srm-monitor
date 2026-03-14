mod synology;
use anyhow::Result;
use serde::Deserialize;

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

fn main() -> Result<()> {
    let creds = match read_credentials() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to read credentials: {}", e);
            return Err(e);
        }
    };

    let synology = match synology::Synology::new(&creds.username, &creds.password) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to create Synology session: {}", e);
            return Err(e);
        }
    };

    let (band, rx_bps, tx_bps) = match synology.fetch_avg_rates(8) {
        Ok(rates) => rates,
        Err(e) => {
            eprintln!("Failed to fetch average rates: {}", e);
            return Err(e);
        }
    };

    println!("Selected Band: {}", band);
    println!("Avg TX Rate: {}", format_bps(tx_bps));
    println!("Avg RX Rate: {}", format_bps(rx_bps));

    // At this point you can store (band, rx_bps, tx_bps) in your DB
    Ok(())
}
