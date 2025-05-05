use once_cell::sync::Lazy;
use serde::Deserialize;
use std::sync::RwLock;
use tracing::debug;

use crate::decoration::{CompiledFormatter, DecorationFormatter};

#[derive(Default, Debug, Clone)]
pub struct Config {
    pub decoration_formatter: CompiledFormatter,
    pub audit: AuditConfig,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AuditConfig {
    #[serde(default)]
    pub disabled: bool,
    #[serde(default = "default_level")]
    pub level: String,
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            disabled: false,
            level: default_level(),
        }
    }
}

fn default_level() -> String {
    "warning".to_string()
}

#[derive(Default, Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct UserConfig {
    #[serde(default)]
    pub decoration_formatter: DecorationFormatter,
    #[serde(default)]
    pub audit: AuditConfig,
}

pub static GLOBAL_CONFIG: Lazy<RwLock<Config>> = Lazy::new(|| RwLock::new(Config::default()));

pub fn initialize_config(config: UserConfig) {
    let mut global_config = GLOBAL_CONFIG.write().unwrap();
    *global_config = Config {
        decoration_formatter: config.decoration_formatter.compile(),
        audit: config.audit,
    };
    debug!("config {:?}", global_config);
}
