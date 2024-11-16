use std::{collections::HashMap, sync::Arc};

use parking_lot::RwLock;
use tokio::sync::mpsc::{self, Sender};
use tower_lsp::{
    lsp_types::{InlayHint, InlayHintLabel, InlayHintLabelPart, Position, Uri},
    Client,
};

use crate::config::GLOBAL_CONFIG;

use super::{formatted_string, DecorationEvent, InlayHintDecorationRenderer};

type InlayHintDecorationState = HashMap<Uri, HashMap<String, InlayHint>>;

mod inlay_hint_decoration_state {
    use super::*;

    pub fn new() -> Arc<RwLock<InlayHintDecorationState>> {
        Arc::new(RwLock::new(HashMap::new()))
    }

    pub fn upsert(state: &RwLock<InlayHintDecorationState>, uri: &Uri, id: &str, hint: InlayHint) {
        let mut state = state.write();
        let path_map = state.entry(uri.clone()).or_default();
        path_map.insert(id.to_string(), hint);
    }

    pub fn update_range(
        state: &RwLock<InlayHintDecorationState>,
        uri: &Uri,
        id: &str,
        range: tower_lsp::lsp_types::Range,
    ) {
        let mut state = state.write();
        if let Some(path_map) = state.get_mut(uri) {
            if let Some(hint) = path_map.get_mut(id) {
                hint.position = Position::new(range.end.line, range.end.character);
            }
        }
    }

    pub fn remove(state: &RwLock<InlayHintDecorationState>, uri: &Uri, id: &str) {
        let mut state = state.write();
        if let Some(path_map) = state.get_mut(uri) {
            path_map.remove(id);
        }
    }

    pub fn reset(state: &RwLock<InlayHintDecorationState>, uri: &Uri) {
        let mut state = state.write();
        state.remove(uri);
    }

    pub fn list(state: &RwLock<InlayHintDecorationState>, uri: &Uri) -> Vec<InlayHint> {
        let state = state.read();
        state
            .get(uri)
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
    hints: Arc<RwLock<InlayHintDecorationState>>,
}

impl InlayHintDecoration {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            hints: inlay_hint_decoration_state::new(),
        }
    }

    pub fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
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
                    DecorationEvent::DependencyWaiting(path, id, range) => {
                        let hint = InlayHint {
                            position: Position::new(range.end.line, range.end.character),
                            label: InlayHintLabel::LabelParts(vec![InlayHintLabelPart {
                                value: GLOBAL_CONFIG
                                    .read()
                                    .unwrap()
                                    .decoration_formatter
                                    .waiting
                                    .template()
                                    .to_string(),
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
                    DecorationEvent::Reset(uri) => {
                        inlay_hint_decoration_state::reset(&state, &uri);
                    }
                    DecorationEvent::Dependency(path, id, range, p) => {
                        let config = GLOBAL_CONFIG.read().unwrap();
                        let Some(decoration) = formatted_string(&p, &config.decoration_formatter)
                        else {
                            continue;
                        };

                        let hint = InlayHint {
                            position: Position::new(range.end.line, range.end.character),
                            label: InlayHintLabel::String(decoration),
                            kind: None,
                            text_edits: None,
                            tooltip: None,
                            padding_left: Some(true),
                            padding_right: None,
                            data: None,
                        };
                        inlay_hint_decoration_state::upsert(&state, &path, &id, hint);
                    }
                    DecorationEvent::DependencyRangeUpdate(path, id, range) => {
                        inlay_hint_decoration_state::update_range(&state, &path, &id, range);
                    }
                }
                client.inlay_hint_refresh().await.unwrap();
            }
        });
        render_tx
    }

    pub fn remove(&mut self, uri: &Uri, id: &str) {
        inlay_hint_decoration_state::remove(&self.hints, uri, id);
    }

    pub fn reset(&mut self, uri: &Uri) {
        inlay_hint_decoration_state::reset(&self.hints, uri);
    }
}

impl InlayHintDecorationRenderer for InlayHintDecoration {
    fn init(&self) -> Sender<DecorationEvent> {
        self.initialize()
    }

    fn list(&self, uri: &Uri) -> Vec<InlayHint> {
        inlay_hint_decoration_state::list(&self.hints, uri)
    }
}
