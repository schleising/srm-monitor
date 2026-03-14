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
        extract_avg_rates(&mesh, node_id)
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

fn extract_avg_rates(mesh: &MeshNetworkInfoResponse, node_id: i32) -> Result<(String, u64, u64)> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn mesh_with_uplinks(wireless_uplinks: &str) -> MeshNetworkInfoResponse {
        serde_json::from_str(&format!(
            r#"{{
                "data": {{
                    "nodes": [
                        {{
                            "node_id": 8,
                            "uplink": {{
                                "wireless_uplinks": {}
                            }}
                        }}
                    ]
                }}
            }}"#,
            wireless_uplinks
        ))
        .unwrap()
    }

    #[test]
    fn parses_login_response() {
        let login: LoginResponse = serde_json::from_str(
            r#"{"success":true,"data":{"sid":"session-id"}}"#,
        )
        .unwrap();

        assert!(login.success);
        assert_eq!(login.data.sid, "session-id");
    }

    #[test]
    fn extracts_rates_preferring_highest_connected_band() {
        let mesh = mesh_with_uplinks(
            r#"[
                {"avg_rx_rate": 100, "avg_tx_rate": 200, "band": "2.4G", "is_connected": true},
                {"avg_rx_rate": 300, "avg_tx_rate": 400, "band": "5G-2", "is_connected": true},
                {"avg_rx_rate": 500, "avg_tx_rate": 600, "band": "5G-1", "is_connected": true}
            ]"#,
        );

        let rates = extract_avg_rates(&mesh, 8).unwrap();

        assert_eq!(rates, ("5G-1".to_string(), 500, 600));
    }

    #[test]
    fn ignores_disconnected_higher_priority_band() {
        let mesh = mesh_with_uplinks(
            r#"[
                {"avg_rx_rate": 100, "avg_tx_rate": 200, "band": "5G-1", "is_connected": false},
                {"avg_rx_rate": 300, "avg_tx_rate": 400, "band": "5G-2", "is_connected": true}
            ]"#,
        );

        let rates = extract_avg_rates(&mesh, 8).unwrap();

        assert_eq!(rates, ("5G-2".to_string(), 300, 400));
    }

    #[test]
    fn errors_when_node_is_missing() {
        let mesh: MeshNetworkInfoResponse = serde_json::from_str(
            r#"{"data":{"nodes":[]}}"#,
        )
        .unwrap();

        let error = extract_avg_rates(&mesh, 8).unwrap_err();

        assert!(error.to_string().contains("Node 8 not found"));
    }

    #[test]
    fn errors_when_no_connected_uplink_exists() {
        let mesh = mesh_with_uplinks(
            r#"[
                {"avg_rx_rate": 100, "avg_tx_rate": 200, "band": "5G-1", "is_connected": false}
            ]"#,
        );

        let error = extract_avg_rates(&mesh, 8).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("No connected wireless uplinks found for node 8")
        );
    }

    #[test]
    fn ensure_http_ok_accepts_200_and_rejects_other_statuses() {
        assert!(ensure_http_ok("Mesh info API call", 200).is_ok());

        let error = ensure_http_ok("Mesh info API call", 500).unwrap_err();
        assert_eq!(error.to_string(), "Mesh info API call failed with status: 500");
    }

    #[test]
    fn band_priority_orders_known_bands() {
        assert!(band_priority("5G-1") > band_priority("5G-2"));
        assert!(band_priority("5G-2") > band_priority("2.4G"));
        assert_eq!(band_priority("unknown"), 0);
    }

    #[test]
    fn select_connected_uplink_returns_none_when_all_are_disconnected() {
        let mesh = mesh_with_uplinks(
            r#"[
                {"avg_rx_rate": 100, "avg_tx_rate": 200, "band": "2.4G", "is_connected": false},
                {"avg_rx_rate": 300, "avg_tx_rate": 400, "band": "5G-2", "is_connected": false}
            ]"#,
        );

        let node = &mesh.data.nodes[0];
        assert!(select_connected_uplink(node).is_none());
    }
}
