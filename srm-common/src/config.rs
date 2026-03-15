use anyhow::{Context, Result};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::fs;
use std::path::{Path, PathBuf};

pub fn env_or_default_path(env_name: &str, default_path: &str) -> PathBuf {
    std::env::var_os(env_name)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default_path))
}

pub fn env_or_manifest_path(
    env_name: &str,
    default_path: &str,
    manifest_dir: impl AsRef<Path>,
) -> PathBuf {
    std::env::var_os(env_name)
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest_dir.as_ref().join(default_path))
}

pub fn load_toml_file<T: DeserializeOwned>(path: impl AsRef<Path>) -> Result<T> {
    let path = path.as_ref();
    let contents = fs::read_to_string(path)
        .with_context(|| format!("failed to read config file {}", path.display()))?;

    toml::from_str(&contents)
        .with_context(|| format!("failed to parse TOML config {}", path.display()))
}

#[derive(Clone, Debug, Deserialize)]
pub struct MongoSettings {
    pub url: String,
    pub database: String,
    pub collection: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SynologyCredentials {
    pub username: String,
    pub password: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SynologySettings {
    #[serde(default = "default_synology_base_url")]
    pub base_url: String,
    #[serde(default = "default_node_id")]
    pub node_id: i32,
    #[serde(default = "default_poll_interval_secs")]
    pub poll_interval_secs: u64,
    pub credentials: SynologyCredentials,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ServiceConfig {
    pub synology: SynologySettings,
    pub mongodb: MongoSettings,
}

#[derive(Clone, Debug, Deserialize)]
pub struct HttpServerSettings {
    #[serde(default = "default_api_bind_address")]
    pub bind_address: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct WebServerSettings {
    #[serde(default = "default_web_bind_address")]
    pub bind_address: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ApiConfig {
    pub server: HttpServerSettings,
    pub mongodb: MongoSettings,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ApiClientSettings {
    pub base_url: String,
    #[serde(default = "default_refresh_interval_secs")]
    pub refresh_interval_secs: u64,
    #[serde(default = "default_history_start")]
    pub history_start: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GuiConfig {
    pub api: ApiClientSettings,
}

#[derive(Clone, Debug, Deserialize)]
pub struct WebApiSettings {
    #[serde(default = "default_web_api_base_url")]
    pub base_url: String,
    #[serde(default = "default_refresh_interval_secs")]
    pub refresh_interval_secs: u64,
    #[serde(default = "default_history_window_secs")]
    pub history_window_secs: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct WebConfig {
    pub server: WebServerSettings,
    pub api: WebApiSettings,
}

fn default_synology_base_url() -> String {
    "http://192.168.1.1:8000/webapi".to_string()
}

fn default_node_id() -> i32 {
    8
}

fn default_poll_interval_secs() -> u64 {
    30
}

fn default_api_bind_address() -> String {
    "127.0.0.1:6081".to_string()
}

fn default_web_bind_address() -> String {
    "127.0.0.1:6080".to_string()
}

fn default_refresh_interval_secs() -> u64 {
    30
}

fn default_history_start() -> String {
    "1970-01-01T00:00:00Z".to_string()
}

fn default_web_api_base_url() -> String {
    "http://127.0.0.1:6081".to_string()
}

fn default_history_window_secs() -> u64 {
    300
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_manifest_relative_default_when_env_not_set() {
        let env_name = "SRM_TEST_CONFIG_PATH_DEFAULT";
        unsafe {
            std::env::remove_var(env_name);
        }

        let path = env_or_manifest_path(env_name, "config/service.toml", "/tmp/srm-monitor");

        assert_eq!(path, PathBuf::from("/tmp/srm-monitor/config/service.toml"));
    }

    #[test]
    fn env_override_takes_priority() {
        let env_name = "SRM_TEST_CONFIG_PATH_OVERRIDE";
        unsafe {
            std::env::set_var(env_name, "custom/config.toml");
        }

        let path = env_or_manifest_path(env_name, "config/service.toml", "/tmp/srm-monitor");

        assert_eq!(path, PathBuf::from("custom/config.toml"));
        unsafe {
            std::env::remove_var(env_name);
        }
    }
}
