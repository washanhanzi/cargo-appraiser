use once_cell::sync::Lazy;
use serde::Deserialize;
use std::sync::RwLock;
use tracing::info;

use crate::decoration::{CompiledFormatter, DecorationFormatter};

#[derive(Default, Debug, Clone)]
pub struct Config {
    pub decoration_formatter: CompiledFormatter,
}

#[derive(Default, Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct UserConfig {
    #[serde(default)]
    pub decoration_formatter: DecorationFormatter,
}

pub static GLOBAL_CONFIG: Lazy<RwLock<Config>> = Lazy::new(|| RwLock::new(Config::default()));

pub fn initialize_config(config: UserConfig) {
    let mut global_config = GLOBAL_CONFIG.write().unwrap();
    *global_config = Config {
        decoration_formatter: config.decoration_formatter.compile(),
    };
}
