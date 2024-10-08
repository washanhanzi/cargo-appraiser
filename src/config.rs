use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::sync::RwLock;

use crate::decoration::DecorationFormatter;

#[derive(Default, Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    #[serde(flatten)]
    pub renderer: RendererConfig,
}

#[derive(Default, Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RendererConfig {
    #[serde(default)]
    pub decoration_format: DecorationFormatter,
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
