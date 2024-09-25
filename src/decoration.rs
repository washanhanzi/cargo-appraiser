use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::Sender;
use tower_lsp::{
    lsp_types::{InlayHint, Range},
    Client,
};

use crate::entity::Dependency;

pub mod inlay_hint;

#[derive(clap::ValueEnum, Debug, Clone)]
pub enum Renderer {
    #[value(name = "inlayHint")]
    InlayHint,
    #[value(name = "vscode")]
    VSCode,
}

pub trait VSCodeDecorationRenderer: Send + Sync + std::fmt::Debug {
    fn init(&self) -> Sender<DecorationEvent>;
}

pub trait InlayHintDecorationRenderer: Send + Sync + std::fmt::Debug {
    fn init(&self) -> Sender<DecorationEvent>;
    //only work for inlayHint renderer
    fn list(&self, path: &str) -> Vec<InlayHint>;
}

#[derive(Debug)]
pub enum DecorationRenderer {
    VSCode(Box<dyn VSCodeDecorationRenderer>),
    InlayHint(Box<dyn InlayHintDecorationRenderer>),
}

impl DecorationRenderer {
    pub fn new(client: Client, renderer: Renderer) -> Self {
        match renderer {
            Renderer::InlayHint => DecorationRenderer::InlayHint(Box::new(
                inlay_hint::InlayHintDecoration::new(client),
            )),
            Renderer::VSCode => DecorationRenderer::InlayHint(Box::new(
                inlay_hint::InlayHintDecoration::new(client),
            )),
        }
    }
    pub fn init(&self) -> Sender<DecorationEvent> {
        match self {
            DecorationRenderer::VSCode(renderer) => renderer.init(),
            DecorationRenderer::InlayHint(renderer) => renderer.init(),
        }
    }
}

#[derive(Clone)]
pub enum DecorationEvent {
    Reset(String),
    DependencyRemove(String, String),
    DependencyLoading(String, String, Range),
    Dependency(String, String, Range, Dependency),
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
