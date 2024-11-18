use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{self, Sender};
use tower_lsp::{
    lsp_types::{self, Range, Uri},
    Client,
};
use tracing::error;

use crate::config::GLOBAL_CONFIG;

use super::{formatted_string, DecorationEvent, VSCodeDecorationRenderer, VersionDecorationKind};

#[derive(Debug, Clone, Deserialize, Serialize)]
struct CreateDecorationRequest {
    pub uri: Uri,
    pub id: String,
    pub text: String,
    pub kind: VersionDecorationKind,
    pub range: Range,
}

impl lsp_types::request::Request for CreateDecorationRequest {
    type Params = CreateDecorationRequest;
    type Result = ();
    const METHOD: &'static str = "textDocument/decoration/create";
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct UpdateDecorationRangeRequest {
    pub uri: Uri,
    pub id: String,
    pub range: Range,
}

impl lsp_types::request::Request for UpdateDecorationRangeRequest {
    type Params = UpdateDecorationRangeRequest;
    type Result = ();
    const METHOD: &'static str = "textDocument/decoration/updateRange";
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct DeleteDecorationRequest {
    pub uri: Uri,
    pub id: String,
}

impl lsp_types::request::Request for DeleteDecorationRequest {
    type Params = DeleteDecorationRequest;
    type Result = ();
    const METHOD: &'static str = "textDocument/decoration/delete";
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
                    DecorationEvent::DependencyWaiting(uri, id, range) => {
                        let text = GLOBAL_CONFIG
                            .read()
                            .unwrap()
                            .decoration_formatter
                            .waiting
                            .template()
                            .to_string();
                        if let Err(err) = client
                            .send_request::<CreateDecorationRequest>(CreateDecorationRequest {
                                uri,
                                id,
                                text,
                                kind: VersionDecorationKind::NotParsed,
                                range,
                            })
                            .await
                        {
                            error!("create decoration error: {}", err);
                        }
                    }
                    DecorationEvent::DependencyRemove(uri, id) => {
                        if let Err(err) = client
                            .send_request::<DeleteDecorationRequest>(DeleteDecorationRequest {
                                uri,
                                id,
                            })
                            .await
                        {
                            error!("delete decoration error: {}", err);
                        }
                    }
                    DecorationEvent::Reset(uri) => {
                        if let Err(err) = client
                            .send_request::<ResetDecorationRequest>(ResetDecorationRequest { uri })
                            .await
                        {
                            error!("reset decoration error: {}", err);
                        }
                    }
                    DecorationEvent::DependencyRangeUpdate(uri, id, range) => {
                        if let Err(err) = client
                            .send_request::<UpdateDecorationRangeRequest>(
                                UpdateDecorationRangeRequest { uri, id, range },
                            )
                            .await
                        {
                            error!("update decoration range error: {}", err);
                        }
                    }
                    DecorationEvent::Dependency(uri, id, range, p) => {
                        let decoration = {
                            let config = GLOBAL_CONFIG.read().unwrap();
                            formatted_string(&p, &config.decoration_formatter)
                        };
                        let Some((kind, text)) = decoration else {
                            continue;
                        };
                        if let Err(err) = client
                            .send_request::<CreateDecorationRequest>(CreateDecorationRequest {
                                uri,
                                id,
                                text,
                                kind,
                                range,
                            })
                            .await
                        {
                            error!("reset decoration error: {}", err);
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
