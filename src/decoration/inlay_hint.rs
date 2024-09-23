use std::{collections::HashMap, sync::Arc};

use cargo::core::SourceKind;
use parking_lot::RwLock;
use tokio::sync::mpsc::{self, Sender};
use tower_lsp::{
    lsp_types::{InlayHint, InlayHintKind, InlayHintLabel, InlayHintLabelPart, Position, Range},
    Client,
};

use super::{DecorationEvent, InlayHintDecorationRenderer};

type InlayHintDecorationState = HashMap<String, HashMap<String, InlayHint>>;

mod inlay_hint_decoration_state {
    use super::*;

    pub fn new() -> Arc<RwLock<InlayHintDecorationState>> {
        Arc::new(RwLock::new(HashMap::new()))
    }

    pub fn upsert(state: &RwLock<InlayHintDecorationState>, path: &str, id: &str, hint: InlayHint) {
        let mut state = state.write();
        let path_map = state.entry(path.to_string()).or_insert(HashMap::new());
        path_map.insert(id.to_string(), hint);
    }

    pub fn remove(state: &RwLock<InlayHintDecorationState>, path: &str, id: &str) {
        let mut state = state.write();
        if let Some(path_map) = state.get_mut(path) {
            path_map.remove(id);
        }
    }

    pub fn reset(state: &RwLock<InlayHintDecorationState>, path: &str) {
        let mut state = state.write();
        if let Some(path_map) = state.get_mut(path) {
            path_map.clear();
        }
    }

    pub fn list(state: &RwLock<InlayHintDecorationState>, path: &str) -> Vec<InlayHint> {
        let state = state.read();
        state
            .get(path)
            .cloned()
            .unwrap_or_default()
            .values()
            .cloned()
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct InlayHintDecoration {
    client: Client,
    initialized: bool,
    hints: Arc<RwLock<InlayHintDecorationState>>,
}

impl InlayHintDecoration {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            initialized: false,
            hints: inlay_hint_decoration_state::new(),
        }
    }

    pub fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            initialized: self.initialized,
            hints: Arc::clone(&self.hints),
        }
    }

    pub fn initialize(&self) -> Sender<DecorationEvent> {
        let (render_tx, mut render_rx) = mpsc::channel::<DecorationEvent>(64);
        let state = Arc::clone(&self.hints);
        let client = self.client.clone();

        tokio::spawn(async move {
            while let Some(event) = render_rx.recv().await {
                match event {
                    DecorationEvent::DependencyLoading(path, id, range) => {
                        let hint = InlayHint {
                            position: Position::new(range.end.line - 1, range.end.character),
                            label: InlayHintLabel::LabelParts(vec![InlayHintLabelPart {
                                value: "Loading".to_string(),
                                tooltip: None,
                                location: None,
                                command: None,
                            }]),
                            kind: None,
                            text_edits: None,
                            tooltip: None,
                            padding_left: None,
                            padding_right: Some(true),
                            data: None,
                        };
                        inlay_hint_decoration_state::upsert(&state, &path, &id, hint);
                    }
                    DecorationEvent::DependencyRemove(path, id) => {
                        inlay_hint_decoration_state::remove(&state, &path, &id);
                    }
                    DecorationEvent::Reset(path) => {
                        inlay_hint_decoration_state::reset(&state, &path);
                    }
                    DecorationEvent::Dependency(path, id, range, p) => {
                        let display = match p.unresolved.as_ref().unwrap().source_id().kind() {
                            SourceKind::Path => "Local".to_string(),
                            SourceKind::Directory => "Directory".to_string(),
                            _ => {
                                let installed_version =
                                    p.resolved.map(|resolved| resolved.version.to_string());
                                let latest_version =
                                    p.latest_summary.map(|latest| latest.version().to_string());
                                format!(
                                    "{}, {}",
                                    installed_version.unwrap_or("Not Installed".to_string()),
                                    latest_version.unwrap_or_default()
                                )
                            }
                        };

                        let hint = InlayHint {
                            position: Position::new(range.end.line - 1, range.end.character),
                            label: InlayHintLabel::String(display),
                            kind: None,
                            text_edits: None,
                            tooltip: None,
                            padding_left: Some(true),
                            padding_right: None,
                            data: None,
                        };
                        inlay_hint_decoration_state::upsert(&state, &path, &id, hint);
                    }
                }
                client.inlay_hint_refresh().await.unwrap();
            }
        });
        render_tx
    }

    pub fn remove(&mut self, path: &str, id: &str) {
        inlay_hint_decoration_state::remove(&self.hints, &path, id);
    }

    pub fn reset(&mut self, path: &str) {
        inlay_hint_decoration_state::reset(&self.hints, &path);
    }
}

impl InlayHintDecorationRenderer for InlayHintDecoration {
    fn init(&self) -> Sender<DecorationEvent> {
        self.initialize()
    }

    fn list(&self, path: &str) -> Vec<InlayHint> {
        inlay_hint_decoration_state::list(&self.hints, &path)
    }
}
