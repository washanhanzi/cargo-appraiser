use once_cell::sync::Lazy;
use serde::Deserialize;
use std::sync::RwLock;
use tracing::debug;

use crate::decoration::{CompiledFormatter, DecorationFormatter};

#[derive(Default, Debug, Clone)]
pub struct Config {
    pub decoration_formatter: CompiledFormatter,
    pub audit: AuditConfig,
    pub crates_io: CratesIoConfig,
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

/// User-provided crates.io configuration (from LSP initialization options)
#[derive(Debug, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct CratesIoUserConfig {
    /// Sparse index URL. None = use default, empty string = disabled
    pub sparse_index_url: Option<String>,
    /// API URL for search. None = use default, empty string = disabled
    pub api_url: Option<String>,
}

/// Runtime crates.io configuration
#[derive(Debug, Clone)]
pub struct CratesIoConfig {
    /// Sparse index URL. None = feature disabled
    pub sparse_index_url: Option<String>,
    /// API URL for search. None = feature disabled
    pub api_url: Option<String>,
}

impl Default for CratesIoConfig {
    fn default() -> Self {
        Self {
            sparse_index_url: Some("https://index.crates.io".to_string()),
            api_url: Some("https://crates.io/api/v1/crates".to_string()),
        }
    }
}

/// Resolve URL from user input:
/// - None -> Some(default) (use default)
/// - Some("") -> None (disabled)
/// - Some(url) -> Some(url without trailing slash)
fn resolve_url(input: Option<String>, default: &str) -> Option<String> {
    match input {
        None => Some(default.to_string()),
        Some(s) if s.is_empty() => None,
        Some(s) => Some(s.trim_end_matches('/').to_string()),
    }
}

#[derive(Default, Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct UserConfig {
    #[serde(default)]
    pub decoration_formatter: DecorationFormatter,
    #[serde(default)]
    pub audit: AuditConfig,
    #[serde(default)]
    pub crates_io: CratesIoUserConfig,
}

pub static GLOBAL_CONFIG: Lazy<RwLock<Config>> = Lazy::new(|| RwLock::new(Config::default()));

pub fn initialize_config(config: UserConfig) {
    let mut global_config = GLOBAL_CONFIG.write().unwrap();
    *global_config = Config {
        decoration_formatter: config.decoration_formatter.compile(),
        audit: config.audit,
        crates_io: CratesIoConfig {
            sparse_index_url: resolve_url(
                config.crates_io.sparse_index_url,
                "https://index.crates.io",
            ),
            api_url: resolve_url(
                config.crates_io.api_url,
                "https://crates.io/api/v1/crates",
            ),
        },
    };
    debug!("config {:?}", global_config);
}
