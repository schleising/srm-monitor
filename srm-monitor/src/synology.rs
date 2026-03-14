use anyhow::{anyhow, Result};
use serde::Deserialize;

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
    is_connected: bool,
}

pub struct Synology {
    sid: String,
}

impl Synology {
    pub fn new(username: &str, password: &str) -> Result<Self> {
        let login = Self::login(username, password)?;
        if !login.success {
            return Err(anyhow!("Login unsuccessful"));
        }

        Ok(Self {
            sid: login.data.sid,
        })
    }

    pub fn fetch_avg_rates(&self, node_id: i32) -> Result<(String, u64, u64)> {
        let mesh = self.fetch_mesh_network_info()?;
        let node = mesh
            .data
            .nodes
            .iter()
            .find(|candidate| candidate.node_id == node_id)
            .ok_or_else(|| anyhow!("Node {} not found", node_id))?;

        let uplink = select_connected_uplink(node)
            .ok_or_else(|| anyhow!("No connected wireless uplinks found for node {}", node_id))?;

        Ok((uplink.band.clone(), uplink.avg_rx_rate, uplink.avg_tx_rate))
    }

    fn logout(&self) -> Result<()> {
        let response = ureq::get(&format!("{}{}", SYNOLOGY_API_BASE_URL, SYNOLOGY_AUTH_URL))
            .query("api", SYNOLOGY_AUTH_API)
            .query("version", SYNOLOGY_AUTH_VERSION.to_string())
            .query("method", SYNOLOGY_AUTH_LOGOUT_METHOD)
            .header("Cookie", &self.session_cookie())
            .call()?;
        ensure_http_ok("Logout API call", response.status().into())
    }

    fn login(username: &str, password: &str) -> Result<LoginResponse> {
        let mut response = ureq::get(&format!("{}{}", SYNOLOGY_API_BASE_URL, SYNOLOGY_AUTH_URL))
            .query("api", SYNOLOGY_AUTH_API)
            .query("version", SYNOLOGY_AUTH_VERSION.to_string())
            .query("method", SYNOLOGY_AUTH_LOGIN_METHOD)
            .query("account", username)
            .query("passwd", password)
            .call()?;

        ensure_http_ok("Login API call", response.status().into())?;
        Ok(response.body_mut().read_json()?)
    }

    fn fetch_mesh_network_info(&self) -> Result<MeshNetworkInfoResponse> {
        let mut response = ureq::get(&format!("{}{}", SYNOLOGY_API_BASE_URL, SYNOLOGY_ENTRY_URL))
            .query("api", SYNOLOGY_MESH_NETWORK_INFO_API)
            .query("version", SYNOLOGY_MESH_NETWORK_INFO_VERSION.to_string())
            .query("method", SYNOLOGY_MESH_NETWORK_INFO_METHOD)
            .header("Cookie", &self.session_cookie())
            .call()?;
        ensure_http_ok("Mesh info API call", response.status().into())?;
        Ok(response.body_mut().read_json()?)
    }

    fn session_cookie(&self) -> String {
        format!("id={}", self.sid)
    }
}

fn ensure_http_ok(context: &str, status: u16) -> Result<()> {
    if status == 200 {
        return Ok(());
    }

    Err(anyhow!("{} failed with status: {}", context, status))
}

fn select_connected_uplink(node: &Node) -> Option<&WirelessUplink> {
    // Prefer the best connected uplink by band priority so disconnected entries never win.
    node.uplink
        .wireless_uplinks
        .iter()
        .filter(|uplink| uplink.is_connected)
        .max_by_key(|uplink| band_priority(&uplink.band))
}

fn band_priority(band: &str) -> u8 {
    match band {
        "2.4G" => 1,
        "5G-2" => 2,
        "5G-1" => 3,
        _ => 0,
    }
}

impl Drop for Synology {
    fn drop(&mut self) {
        if let Err(e) = self.logout() {
            eprintln!("Warning: failed to logout from Synology session: {}", e);
        }
    }
}
