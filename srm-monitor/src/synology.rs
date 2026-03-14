use serde::Deserialize;
use std::error::Error;

const SYNOLOGY_API_BASE_URL: &str = "http://192.168.1.1:8000/webapi";
const SYNOLOGY_AUTH_API: &str = "SYNO.API.Auth";
const SYNOLOGY_AUTH_URL: &str = "/auth.cgi";
const SYNOLOGY_AUTH_VERSION: u8 = 3;
const SYNOLOGY_AUTH_LOGIN_METHOD: &str = "login";
const SYNOLOGY_AUTH_LOGOUT_METHOD: &str = "logout";
const SYNOLOGY_ENTRY_URL: &str = "/entry.cgi";
const SYNOLOGY_MESH_NETWORK_INFO_API: &str = "SYNO.Mesh.Network.Info";
const SYNOLOGY_MESH_NETWORK_INFO_VERSION: u8 = 3;
const SYNOLOGY_MESH_NETWORK_INFO_METHOD: &str = "get";

#[derive(Debug, Deserialize)]
struct Cookie {
    sid: String,
}

#[derive(Debug, Deserialize)]
struct LoginResponse {
    success: bool,
    data: Cookie,
}

#[derive(Debug, Deserialize)]
struct MeshNetworkInfoResponse {
    data: MeshData,
}

#[derive(Debug, Deserialize)]
struct MeshData {
    nodes: Vec<Node>,
}

#[derive(Debug, Deserialize)]
struct Node {
    node_id: i32,
    uplink: Uplink,
}

#[derive(Debug, Deserialize)]
struct Uplink {
    wireless_uplinks: Vec<WirelessUplink>,
}

#[derive(Debug, Deserialize)]
struct WirelessUplink {
    avg_rx_rate: u64,
    avg_tx_rate: u64,
    band: String,
}

pub struct Synology {
    sid: String,
}

impl Synology {
    pub fn new(username: &str, password: &str) -> Result<Self, Box<dyn Error>> {
        let mut resp = ureq::get(&format!("{}{}", SYNOLOGY_API_BASE_URL, SYNOLOGY_AUTH_URL))
            .query("api", SYNOLOGY_AUTH_API)
            .query("version", SYNOLOGY_AUTH_VERSION.to_string())
            .query("method", SYNOLOGY_AUTH_LOGIN_METHOD)
            .query("account", username)
            .query("passwd", password)
            .call()?;

        if resp.status() != 200 {
            return Err(format!("Login API call failed with status: {}", resp.status()).into());
        }

        let login: LoginResponse = resp.body_mut().read_json()?;
        if !login.success {
            return Err("Login unsuccessful".into());
        }

        Ok(Self { sid: login.data.sid })
    }

    pub fn fetch_avg_rates(&self, node_id: i32, band: &str) -> Result<(f64, f64), Box<dyn Error>> {
        let sid_hdr = format!("id={}", self.sid);
        let mut resp = ureq::get(&format!("{}{}", SYNOLOGY_API_BASE_URL, SYNOLOGY_ENTRY_URL))
            .query("api", SYNOLOGY_MESH_NETWORK_INFO_API)
            .query("version", SYNOLOGY_MESH_NETWORK_INFO_VERSION.to_string())
            .query("method", SYNOLOGY_MESH_NETWORK_INFO_METHOD)
            .header("Cookie", &sid_hdr)
            .call()?;

        if resp.status() != 200 {
            return Err(format!("Mesh info API call failed with status: {}", resp.status()).into());
        }

        let mesh: MeshNetworkInfoResponse = resp.body_mut().read_json()?;

        if let Some(node) = mesh.data.nodes.iter().find(|n| n.node_id == node_id) {
            if let Some(uplink) = node.uplink.wireless_uplinks.iter().find(|u| u.band == band) {
                let rx_gbps = uplink.avg_rx_rate as f64 / 1_000_000_000.0;
                let tx_gbps = uplink.avg_tx_rate as f64 / 1_000_000_000.0;
                return Ok((rx_gbps, tx_gbps));
            }

            return Err(format!("Band {} not found for node {}", band, node_id).into());
        }

        Err(format!("Node {} not found", node_id).into())
    }

    fn logout(&self) -> Result<(), Box<dyn Error>> {
        let sid_hdr = format!("id={}", self.sid);
        let resp = ureq::get(&format!("{}{}", SYNOLOGY_API_BASE_URL, SYNOLOGY_AUTH_URL))
            .query("api", SYNOLOGY_AUTH_API)
            .query("version", SYNOLOGY_AUTH_VERSION.to_string())
            .query("method", SYNOLOGY_AUTH_LOGOUT_METHOD)
            .header("Cookie", &sid_hdr)
            .call()?;

        if resp.status() != 200 {
            return Err(format!("Logout API call failed with status: {}", resp.status()).into());
        }

        Ok(())
    }
}

impl Drop for Synology {
    fn drop(&mut self) {
        if let Err(e) = self.logout() {
            eprintln!("Warning: failed to logout from Synology session: {}", e);
        }
    }
}
