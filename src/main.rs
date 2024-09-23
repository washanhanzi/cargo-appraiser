use controller::{Appraiser, CargoDocumentEvent, CargoTomlPayload};
use decoration::DecorationRenderer;
use tokio::sync::mpsc::Sender;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

mod controller;
mod decoration;
mod entity;
mod usecase;

#[derive(Debug)]
struct CargoAppraiser {
    client: Client,
    tx: Sender<CargoDocumentEvent>,
    render: DecorationRenderer,
}

#[tower_lsp::async_trait]
impl LanguageServer for CargoAppraiser {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                text_document_sync: Some(TextDocumentSyncCapability::Options(
                    TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: None,
                        will_save: None,
                        will_save_wait_until: None,
                        save: Some(TextDocumentSyncSaveOptions::SaveOptions(SaveOptions {
                            include_text: Some(true),
                        })),
                    },
                )),
                workspace: Some(WorkspaceServerCapabilities {
                    workspace_folders: None,
                    file_operations: None,
                }),
                inlay_hint_provider: Some(OneOf::Right(
                    InlayHintServerCapabilities::RegistrationOptions(
                        InlayHintRegistrationOptions {
                            inlay_hint_options: InlayHintOptions {
                                resolve_provider: Some(true),
                                work_done_progress_options: WorkDoneProgressOptions::default(),
                            },
                            text_document_registration_options: TextDocumentRegistrationOptions {
                                document_selector: Some(vec![DocumentFilter {
                                    language: Some("toml".to_string()),
                                    pattern: Some("**/Cargo.toml".to_string()),
                                    scheme: None,
                                }]),
                            },
                            static_registration_options: Default::default(),
                        },
                    ),
                )),
                diagnostic_provider: Some(DiagnosticServerCapabilities::RegistrationOptions(
                    DiagnosticRegistrationOptions {
                        text_document_registration_options: TextDocumentRegistrationOptions {
                            document_selector: Some(vec![DocumentFilter {
                                language: Some("toml".to_string()),
                                pattern: Some("**/Cargo.toml".to_string()),
                                scheme: None,
                            }]),
                        },
                        diagnostic_options: DiagnosticOptions {
                            workspace_diagnostics: false,
                            inter_file_dependencies: false,
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                )),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "server initialized again!")
            .await;
    }

    async fn diagnostic(
        &self,
        params: DocumentDiagnosticParams,
    ) -> Result<DocumentDiagnosticReportResult> {
        Ok(DocumentDiagnosticReportResult::Partial(
            DocumentDiagnosticReportPartialResult {
                related_documents: None,
            },
        ))
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let path = params.text_document.uri.path().to_string();
        if !path.ends_with("Cargo.toml") {
            return;
        };
        eprintln!("did open: {}", path);
        self.tx
            .send(CargoDocumentEvent::Opened(CargoTomlPayload {
                path,
                text: params.text_document.text,
            }))
            .await
            .unwrap();
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let path = params.text_document.uri.path().to_string();
        if !path.ends_with("Cargo.toml") {
            return;
        };
        eprintln!("did close: {}", path);
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let path = params.text_document.uri.path().to_string();
        if !path.ends_with("Cargo.toml") {
            return;
        };
        let diagnostic = Diagnostic {
            range: Range {
                start: Position::new(5, 0),
                end: Position::new(5, 80),
            },
            severity: Some(DiagnosticSeverity::INFORMATION),
            code: None,
            code_description: None,
            source: Some("example-lsp".to_string()),
            message: "This line is decorated".to_string(),
            tags: None,
            data: None,
            related_information: Some(vec![DiagnosticRelatedInformation {
                location: Location {
                    uri: params.text_document.uri.clone(),
                    range: Range {
                        start: Position::new(5, 0),
                        end: Position::new(5, 80),
                    },
                },
                message: "This is the text to display without underlining".to_string(),
            }]),
        };

        // Create and publish the diagnostics
        let pub_params = PublishDiagnosticsParams {
            uri: params.text_document.uri.clone(),
            diagnostics: vec![diagnostic],
            version: None,
        };

        self.client
            .publish_diagnostics(pub_params.uri, pub_params.diagnostics, pub_params.version)
            .await;

        if let Some(text) = params.text {
            self.tx
                .send(CargoDocumentEvent::Saved(CargoTomlPayload { path, text }))
                .await
                .unwrap();

            // self.client
            //     .log_message(MessageType::INFO, "Cargo.toml saved. ")
            //     .await;
        };
    }

    async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
        let path = params.text_document.uri.path().to_string();
        if !path.ends_with("Cargo.toml") {
            return Ok(None);
        };

        if let DecorationRenderer::InlayHint(renderer) = &self.render {
            Ok(Some(renderer.list(&path)))
        } else {
            Ok(None)
        }
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let position = params.text_document_position_params.position;
        let hover_text = format!(
            "Hover request at line {}, character {}",
            position.line, position.character
        );
        Ok(Some(Hover {
            contents: HoverContents::Scalar(MarkedString::String(hover_text)),
            range: None,
        }))
    }
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| {
        let render = DecorationRenderer::new(client.clone());
        let render_tx = render.init();

        let state = Appraiser::new(client.clone(), render_tx.clone());
        let tx = state.initialize();

        CargoAppraiser { client, tx, render }
    });

    Server::new(stdin, stdout, socket).serve(service).await;
}
