use tokio::sync::mpsc::Sender;
use tower_lsp::{
    lsp_types::{InlayHint, Range},
    Client,
};

use crate::entity::Dependency;

pub mod inlay_hint;

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
    pub fn new(client: Client) -> Self {
        DecorationRenderer::InlayHint(Box::new(inlay_hint::InlayHintDecoration::new(client)))
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
