use std::path::Path;

use clap::Parser;
use config::{initialize_config, UserConfig};
use controller::{Appraiser, CargoDocumentEvent, CargoTomlPayload, ClientCapability};
use decoration::{DecorationRenderer, Renderer};
use entity::{supported_commands, CARGO};
use serde_json::Value;
use tokio::sync::{mpsc::Sender, oneshot};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{LanguageServer, LspService, Server};
use tracing::{error, info};

mod config;
mod controller;
mod decoration;
mod entity;
mod usecase;

/// Run the resolve subcommand - resolves dependencies and outputs JSON to stdout.
/// This is designed to be called as a subprocess by the LSP server.
fn run_resolve_worker(manifest_path: &str) {
    use cargo_parser::CargoIndex;

    match CargoIndex::resolve_direct(Path::new(manifest_path)) {
        Ok(index) => {
            let serializable = index.to_serializable();
            if let Err(e) = serde_json::to_writer(std::io::stdout(), &serializable) {
                eprintln!("Failed to serialize output: {}", e);
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    }
}

#[derive(Debug)]
struct CargoAppraiser {
    tx: Sender<CargoDocumentEvent>,
    render: DecorationRenderer,
    cargo_path: Option<String>,
}

#[tower_lsp::async_trait]
impl LanguageServer for CargoAppraiser {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        //init config
        let config: UserConfig = params
            .initialization_options
            .map(serde_json::from_value)
            .and_then(|v| v.ok())
            .unwrap_or_default();
        initialize_config(config);

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                execute_command_provider: Some(ExecuteCommandOptions {
                    commands: supported_commands(),
                    ..Default::default()
                }),
                definition_provider: Some(OneOf::Left(true)),
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
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        info!("cargo-appraiser server initialized!");
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        if !uri.path().as_str().ends_with("Cargo.toml") {
            return Ok(None);
        };
        //create a once channel with payload Hover
        let (tx, rx) = oneshot::channel();
        if let Err(e) = self
            .tx
            .send(CargoDocumentEvent::Gded(
                uri,
                params.text_document_position_params.position,
                tx,
            ))
            .await
        {
            error!("error sending goto definition event: {}", e);
            return Ok(None);
        };
        match rx.await {
            Ok(gd) => return Ok(gd),
            Err(_) => {
                return Ok(None);
            }
        }
    }

    async fn execute_command(&self, params: ExecuteCommandParams) -> Result<Option<Value>> {
        let Some(cargo_path) = self.cargo_path.as_deref() else {
            return Ok(None);
        };
        match params.command.as_str() {
            CARGO => {
                let cargo_path = cargo_path.to_string();
                let args = params
                    .arguments
                    .iter()
                    .map(|v| v.as_str().unwrap().to_string())
                    .collect::<Vec<_>>();
                //run cargo command with params in a new task
                let command_result = tokio::process::Command::new(cargo_path).args(args).spawn();
                if command_result.is_err() {
                    return Ok(None);
                }
                if let Err(e) = self.tx.send(CargoDocumentEvent::CargoLockChanged).await {
                    error!(
                        "error sending cargo lock changed event from execute command: {}",
                        e
                    );
                }
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        if !uri.path().as_str().ends_with("Cargo.toml") {
            return;
        };
        if let Err(e) = self
            .tx
            .send(CargoDocumentEvent::Opened(CargoTomlPayload {
                uri,
                text: params.text_document.text,
            }))
            .await
        {
            error!("error sending opened event: {}", e);
        };
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
                error!("error sending changed event: {}", e);
            };
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        if !uri.path().as_str().ends_with("Cargo.toml") {
            return;
        };
        if let Err(e) = self.tx.send(CargoDocumentEvent::Closed(uri)).await {
            error!("error sending closed event: {}", e);
        };
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let uri = params.text_document.uri;
        if !uri.path().as_str().ends_with("Cargo.toml") {
            return;
        };

        if let Some(text) = params.text {
            if let Err(e) = self
                .tx
                .send(CargoDocumentEvent::Saved(CargoTomlPayload { uri, text }))
                .await
            {
                error!("error sending saved event: {}", e);
            };
        };
    }

    async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
        let uri = params.text_document.uri;
        if !uri.path().as_str().ends_with("Cargo.toml") {
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
        let uri = params.text_document_position.text_document.uri;
        if !uri.path().as_str().ends_with("Cargo.toml") {
            return Ok(None);
        };
        let (tx, rx) = oneshot::channel();
        if let Err(e) = self
            .tx
            .send(CargoDocumentEvent::Completion(
                uri,
                params.text_document_position.position,
                tx,
            ))
            .await
        {
            error!("error sending completion event: {}", e);
            return Ok(None);
        };
        match rx.await {
            Ok(completion) => return Ok(completion),
            Err(_) => {
                return Ok(None);
            }
        }
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let uri = params.text_document.uri;
        if !uri.path().as_str().ends_with("Cargo.toml") {
            return Ok(None);
        };
        let (tx, rx) = oneshot::channel();
        if let Err(e) = self
            .tx
            .send(CargoDocumentEvent::CodeAction(uri, params.range, tx))
            .await
        {
            error!("error sending code action event: {}", e);
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
        if !uri.path().as_str().ends_with("Cargo.toml") {
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
            error!("error sending hover event: {}", e);
            return Ok(None);
        };
        match rx.await {
            Ok(hover) => return Ok(hover),
            Err(_) => {
                return Ok(None);
            }
        }
    }

    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        //check params.changes's item, if it end with "Cargo.lock"
        for change in params.changes {
            if change.uri.path().as_str().ends_with("Cargo.lock") {
                //send refresh event
                if let Err(e) = self.tx.send(CargoDocumentEvent::CargoLockChanged).await {
                    error!("error sending cargo lock changed event: {}", e);
                }
            }
        }
    }

    async fn did_change_configuration(&self, params: DidChangeConfigurationParams) {
        info!("did change configuration: {}", params.settings);
    }
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    ///"inlayHint" or "vscode". "inlayHint" is for lsp inlay hints and "vscode" is for vscode decorations
    #[arg(short, long, value_enum)]
    renderer: Renderer,
    ///stdio transport. now only work with stdio transport
    #[arg(short, long, default_value = "true")]
    stdio: bool,
    ///list of supported client capabilities (e.g., "readFile")
    #[arg(short = 'c', long, value_delimiter = ',')]
    client_capabilities: Option<Vec<ClientCapability>>,
}

#[tokio::main]
async fn main() {
    // Check for resolve subcommand before parsing clap args
    // This allows us to use a simple argument format for the worker subprocess
    let raw_args: Vec<String> = std::env::args().collect();
    if raw_args.get(1).map(|s| s.as_str()) == Some("resolve") {
        let manifest_path = raw_args
            .get(2)
            .expect("manifest path required for resolve subcommand");
        run_resolve_worker(manifest_path);
        return;
    }

    // Parse command-line arguments for LSP mode
    let args = Args::parse();

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    //logging
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let cargo_path = executable_path_finder::find_with_cargo_home("cargo").map(|p| p.to_string());
    let (service, socket) = LspService::new(|client| {
        let render = DecorationRenderer::new(client.clone(), args.renderer);
        let render_tx = render.init();

        let state = Appraiser::new(
            client.clone(),
            render_tx.clone(),
            args.client_capabilities.as_deref(),
            cargo_path.clone().unwrap_or("cargo".to_string()),
        );
        let tx = state.initialize();

        CargoAppraiser {
            tx,
            render,
            cargo_path,
        }
    });

    Server::new(stdin, stdout, socket).serve(service).await;
}
