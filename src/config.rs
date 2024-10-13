use once_cell::sync::Lazy;
use serde::Deserialize;
use std::sync::RwLock;

use crate::decoration::DecorationFormatter;

#[derive(Default, Debug, Deserialize, Clone)]
pub struct Config {
    #[serde(flatten)]
    pub renderer: RendererConfig,
}

#[derive(Default, Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RendererConfig {
    #[serde(default)]
    pub decoration_formatter: DecorationFormatter,
}

pub static GLOBAL_CONFIG: Lazy<RwLock<Config>> = Lazy::new(|| RwLock::new(Config::default()));

pub fn initialize_config(mut config: Config) {
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
