use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::sync::RwLock;

#[derive(Default, Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    #[serde(flatten)]
    pub renderer: RendererConfig,
}

#[derive(Default, Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RendererConfig {
    #[serde(default)]
    pub decoration_format: DecorationFormat,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DecorationFormat {
    #[serde(default = "default_latest")]
    pub latest: String,
    #[serde(default = "default_local")]
    pub local: String,
    #[serde(default = "default_not_installed")]
    pub not_installed: String,
    #[serde(default = "default_loading")]
    pub loading: String,
    #[serde(default = "default_mixed_upgradeable")]
    pub mixed_upgradeable: String,
    #[serde(default = "default_compatible_latest")]
    pub compatible_latest: String,
    #[serde(default = "default_noncompatible_latest")]
    pub noncompatible_latest: String,
}

impl Default for DecorationFormat {
    fn default() -> Self {
        Self {
            latest: default_latest(),
            compatible_latest: default_compatible_latest(),
            local: default_local(),
            noncompatible_latest: default_noncompatible_latest(),
            not_installed: default_not_installed(),
            loading: default_loading(),
            mixed_upgradeable: default_mixed_upgradeable(),
        }
    }
}

fn default_latest() -> String {
    "✅ {{installed}}".to_string()
}

fn default_mixed_upgradeable() -> String {
    "🚀🔒 {{installed}} -> {{latest_matched}},  {{latest}}".to_string()
}

fn default_compatible_latest() -> String {
    "🚀 {{installed}} -> {{latest}}".to_string()
}

fn default_noncompatible_latest() -> String {
    "🔒 {{installed}}, {{latest}}".to_string()
}

fn default_not_installed() -> String {
    "Not installed".to_string()
}

fn default_loading() -> String {
    "Loading...".to_string()
}

fn default_local() -> String {
    "Local".to_string()
}

pub static GLOBAL_CONFIG: Lazy<RwLock<Config>> = Lazy::new(|| RwLock::new(Config::default()));

pub fn initialize_config(config: Config) {
    let mut global_config = GLOBAL_CONFIG.write().unwrap();
    *global_config = config;
}

pub fn update_config<F>(update_fn: F)
where
    F: FnOnce(&mut Config),
{
    let mut global_config = GLOBAL_CONFIG.write().unwrap();
    update_fn(&mut global_config);
}
