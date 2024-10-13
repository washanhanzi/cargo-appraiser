use clap::{arg, command, Parser};
use config::{initialize_config, Config};
use controller::{Appraiser, CargoDocumentEvent, CargoTomlPayload};
use decoration::{DecorationRenderer, Renderer};
use tokio::sync::{mpsc::Sender, oneshot};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

mod config;
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
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        //init config
        let config: Config = params
            .initialization_options
            .map(serde_json::from_value)
            .and_then(|v| v.ok())
            .unwrap_or_default();
        initialize_config(config);

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![
                        ".".to_string(),
                        "+".to_string(),
                        "-".to_string(),
                        "'".to_string(),
                        "\"".to_string(),
                        "0".to_string(),
                        "1".to_string(),
                        "2".to_string(),
                        "3".to_string(),
                        "4".to_string(),
                        "5".to_string(),
                        "6".to_string(),
                        "7".to_string(),
                        "8".to_string(),
                        "9".to_string(),
                    ]),
                    ..Default::default()
                }),
                text_document_sync: Some(TextDocumentSyncCapability::Options(
                    TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: Some(TextDocumentSyncKind::FULL),
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
                inlay_hint_provider: Some(OneOf::Left(true)),
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
            .log_message(MessageType::INFO, "cargo-appraiser server initialized!")
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
        let uri = params.text_document.uri;
        if !uri.path().ends_with("Cargo.toml") {
            return;
        };
        self.tx
            .send(CargoDocumentEvent::Opened(CargoTomlPayload {
                uri,
                text: params.text_document.text,
            }))
            .await
            .unwrap();
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        for change in params.content_changes {
            if let Err(e) = self
                .tx
                .send(CargoDocumentEvent::Changed(CargoTomlPayload {
                    uri: params.text_document.uri.clone(),
                    text: change.text,
                }))
                .await
            {
                eprintln!("error sending changed event: {}", e);
            };
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        if !uri.path().ends_with("Cargo.toml") {
            return;
        };
        self.tx.send(CargoDocumentEvent::Closed(uri)).await.unwrap();
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let uri = params.text_document.uri;
        if !uri.path().ends_with("Cargo.toml") {
            return;
        };

        if let Some(text) = params.text {
            if let Err(e) = self
                .tx
                .send(CargoDocumentEvent::Saved(CargoTomlPayload { uri, text }))
                .await
            {
                eprintln!("error sending saved event: {}", e);
            };
        };
    }

    async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
        let uri = params.text_document.uri;
        if !uri.path().ends_with("Cargo.toml") {
            return Ok(None);
        };

        if let DecorationRenderer::InlayHint(renderer) = &self.render {
            Ok(Some(renderer.list(&uri)))
        } else {
            //disable for non inlay hint renderer
            Ok(None)
        }
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        Ok(None)
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let uri = params.text_document.uri;
        let path = uri.path().to_string();
        if !path.ends_with("Cargo.toml") {
            return Ok(None);
        };
        let (tx, rx) = oneshot::channel();
        if let Err(e) = self
            .tx
            .send(CargoDocumentEvent::CodeAction(uri, params.range, tx))
            .await
        {
            self.client
                .log_message(
                    MessageType::ERROR,
                    &format!("error sending code action event: {}", e),
                )
                .await;
            return Ok(None);
        };
        match rx.await {
            Ok(code_action) => return Ok(Some(code_action)),
            Err(_) => {
                return Ok(None);
            }
        }
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let path = uri.path().to_string();
        if !path.ends_with("Cargo.toml") {
            return Ok(None);
        };
        //create a once channel with payload Hover
        let (tx, rx) = oneshot::channel();
        if let Err(e) = self
            .tx
            .send(CargoDocumentEvent::Hovered(
                uri,
                params.text_document_position_params.position,
                tx,
            ))
            .await
        {
            self.client
                .log_message(
                    MessageType::ERROR,
                    &format!("error sending hover event: {}", e),
                )
                .await;
            return Ok(None);
        };
        match rx.await {
            Ok(hover) => return Ok(Some(hover)),
            Err(_) => {
                return Ok(None);
            }
        }
    }

    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        //check params.changes's item, if it end with "Cargo.lock"
        for change in params.changes {
            if change.uri.path().ends_with("Cargo.lock") {
                //send refresh event
                self.tx
                    .send(CargoDocumentEvent::CargoLockChanged)
                    .await
                    .unwrap();
            }
        }
    }
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    ///"inlayHint" or "vscode". "inlayHint" is for lsp inlay hints and "vscode" is for vscode decorations
    #[arg(short, long, value_enum)]
    renderer: Renderer,
    ///delay(milliseconds) for cargo to resolve dependencies after a document change event, default is 3000
    #[arg(short, long, default_value = "3000")]
    delay: u64,
    ///stdio transport. now only work with stdio transport
    #[arg(short, long, default_value = "true")]
    stdio: bool,
}

#[tokio::main]
async fn main() {
    // Parse command-line arguments
    let args = Args::parse();

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| {
        let render = DecorationRenderer::new(client.clone(), args.renderer);
        let render_tx = render.init();

        let state = Appraiser::new(client.clone(), render_tx.clone(), args.delay);
        let tx = state.initialize();

        CargoAppraiser { client, tx, render }
    });

    Server::new(stdin, stdout, socket).serve(service).await;
}
