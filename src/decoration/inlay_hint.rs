use std::{collections::HashMap, sync::Arc};

use parking_lot::RwLock;
use tokio::sync::mpsc::{self, Sender};
use tower_lsp::{
    lsp_types::{InlayHint, InlayHintLabel, Position, Uri},
    Client,
};
use tracing::error;

use crate::config::GLOBAL_CONFIG;

use super::{formatted_string, DecorationEvent, DecorationState, InlayHintDecorationRenderer};

type InlayHintDecorationState = HashMap<Uri, Vec<InlayHint>>;

mod inlay_hint_decoration_state {
    use super::*;

    pub fn new() -> Arc<RwLock<InlayHintDecorationState>> {
        Arc::new(RwLock::new(HashMap::new()))
    }

    pub fn update(state: &RwLock<InlayHintDecorationState>, uri: &Uri, hints: Vec<InlayHint>) {
        let mut state = state.write();
        state.insert(uri.clone(), hints);
    }

    pub fn reset(state: &RwLock<InlayHintDecorationState>, uri: &Uri) {
        let mut state = state.write();
        state.remove(uri);
    }

    pub fn list(state: &RwLock<InlayHintDecorationState>, uri: &Uri) -> Vec<InlayHint> {
        let state = state.read();
        state.get(uri).cloned().unwrap_or_default()
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

    pub fn initialize(&self) -> Sender<DecorationEvent> {
        let (render_tx, mut render_rx) = mpsc::channel::<DecorationEvent>(64);
        let state = Arc::clone(&self.hints);
        let client = self.client.clone();

        tokio::spawn(async move {
            while let Some(event) = render_rx.recv().await {
                match event {
                    DecorationEvent::Reset(uri) => {
                        inlay_hint_decoration_state::reset(&state, &uri);
                    }
                    DecorationEvent::Update(uri, items) => {
                        let config = GLOBAL_CONFIG.read().unwrap();
                        let hints: Vec<InlayHint> = items
                            .into_iter()
                            .filter_map(|item| {
                                let (text, padding_left) = match &item.state {
                                    DecorationState::Waiting => (
                                        config.decoration_formatter.waiting.template().to_string(),
                                        false,
                                    ),
                                    DecorationState::Resolved { dep, resolved } => {
                                        let (_, text) = formatted_string(
                                            dep,
                                            resolved.as_ref(),
                                            &config.decoration_formatter,
                                        )?;
                                        (text, true)
                                    }
                                };
                                Some(InlayHint {
                                    position: Position::new(
                                        item.range.end.line,
                                        item.range.end.character,
                                    ),
                                    label: InlayHintLabel::String(text),
                                    kind: None,
                                    text_edits: None,
                                    tooltip: None,
                                    padding_left: Some(padding_left),
                                    padding_right: Some(!padding_left),
                                    data: None,
                                })
                            })
                            .collect();
                        inlay_hint_decoration_state::update(&state, &uri, hints);
                    }
                }
                if let Err(e) = client.inlay_hint_refresh().await {
                    error!("inlay hint refresh error: {}", e);
                }
            }
        });
        render_tx
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
