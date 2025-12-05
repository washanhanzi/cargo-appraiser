use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{self, Sender};
use tower_lsp::{
    lsp_types::{self, Range, Uri},
    Client,
};
use tracing::error;

use crate::config::GLOBAL_CONFIG;

use super::{
    formatted_string, DecorationEvent, DecorationState, VSCodeDecorationRenderer,
    VersionDecorationKind,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
struct DecorationData {
    pub id: String,
    pub text: String,
    pub kind: VersionDecorationKind,
    pub range: Range,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct UpdateDecorationsRequest {
    pub uri: Uri,
    pub decorations: Vec<DecorationData>,
}

impl lsp_types::request::Request for UpdateDecorationsRequest {
    type Params = UpdateDecorationsRequest;
    type Result = ();
    const METHOD: &'static str = "textDocument/decoration/replaceAll";
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ResetDecorationRequest {
    pub uri: Uri,
}

impl lsp_types::request::Request for ResetDecorationRequest {
    type Params = ResetDecorationRequest;
    type Result = ();
    const METHOD: &'static str = "textDocument/decoration/reset";
}

#[derive(Debug, Clone)]
pub struct VSCodeDecoration {
    client: Client,
}

impl VSCodeDecoration {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    pub fn initialize(&self) -> Sender<DecorationEvent> {
        let (render_tx, mut render_rx) = mpsc::channel::<DecorationEvent>(64);
        let client = self.client.clone();
        tokio::spawn(async move {
            while let Some(event) = render_rx.recv().await {
                match event {
                    DecorationEvent::Reset(uri) => {
                        if let Err(err) = client
                            .send_request::<ResetDecorationRequest>(ResetDecorationRequest { uri })
                            .await
                        {
                            error!("reset decoration error: {}", err);
                        }
                    }
                    DecorationEvent::Update(uri, items) => {
                        let decorations: Vec<DecorationData> = {
                            let config = GLOBAL_CONFIG.read().unwrap();
                            items
                                .into_iter()
                                .filter_map(|item| {
                                    let (kind, text) = match &item.state {
                                        DecorationState::Waiting => (
                                            VersionDecorationKind::NotParsed,
                                            config
                                                .decoration_formatter
                                                .waiting
                                                .template()
                                                .to_string(),
                                        ),
                                        DecorationState::Resolved { dep, resolved } => {
                                            formatted_string(
                                                dep,
                                                resolved.as_ref(),
                                                &config.decoration_formatter,
                                            )?
                                        }
                                    };
                                    Some(DecorationData {
                                        id: item.id,
                                        text,
                                        kind,
                                        range: item.range,
                                    })
                                })
                                .collect()
                        };
                        if let Err(err) = client
                            .send_request::<UpdateDecorationsRequest>(UpdateDecorationsRequest {
                                uri,
                                decorations,
                            })
                            .await
                        {
                            error!("update decorations error: {}", err);
                        }
                    }
                }
            }
        });
        render_tx
    }
}

impl VSCodeDecorationRenderer for VSCodeDecoration {
    fn init(&self) -> Sender<DecorationEvent> {
        self.initialize()
    }
}
