//! Main Appraiser controller - orchestrates cargo dependency resolution and LSP events.

mod document;
mod resolve;

use std::env;

use tokio::sync::mpsc::{self, Sender};
use tower_lsp::Client;
use tracing::{error, trace, warn};

use crate::{decoration::DecorationEvent, usecase::Workspace};

use self::{
    document::{handle_changed, handle_closed, handle_opened_saved, handle_parse},
    resolve::{handle_cargo_lock_changed, handle_cargo_resolved, handle_ready_to_resolve},
};
use super::{
    audit::{handle_audited, AuditController},
    capabilities::{ClientCapabilities, ClientCapability},
    cargo::cargo_resolve,
    code_action::handle_code_action,
    completion::handle_completion,
    context::{AppraiserContext, CargoDocumentEvent, Ctx},
    debouncer::Debouncer,
    diagnostic::{handle_cargo_diagnostic, DiagnosticController},
    gd::handle_gd,
    hover::handle_hover,
};

/// Main Appraiser controller that manages cargo dependency resolution.
///
/// CargoState will run a dedicated task which receives messages from LSP events.
/// The message payload should contain the file content and LSP client.
/// Tracks currently opened cargo.toml files and their revisions.
#[derive(Debug)]
pub struct Appraiser {
    client: Client,
    render_tx: Sender<DecorationEvent>,
    client_capabilities: ClientCapabilities,
    cargo_path: String,
}

impl Appraiser {
    pub fn new(
        client: Client,
        render_tx: Sender<DecorationEvent>,
        client_capabilities: Option<&[ClientCapability]>,
        cargo_path: String,
    ) -> Self {
        let client_capabilities = ClientCapabilities::new(client_capabilities);
        Self {
            client,
            render_tx,
            client_capabilities,
            cargo_path,
        }
    }

    pub fn initialize(&self) -> Sender<CargoDocumentEvent> {
        // Create mpsc channel
        let (tx, mut rx) = mpsc::channel::<CargoDocumentEvent>(64);
        let inner_tx = tx.clone();

        // Cargo tree task
        // Cargo tree channel
        let (cargo_tx, mut cargo_rx) = mpsc::channel::<Ctx>(32);
        let tx_for_cargo = tx.clone();
        tokio::spawn(async move {
            match env::var("PATH") {
                Ok(path_var) => trace!("Current PATH: {}", path_var),
                Err(e) => warn!("Failed to read PATH environment variable: {}", e),
            }

            while let Some(event) = cargo_rx.recv().await {
                match cargo_resolve(&event).await {
                    Ok(output) => {
                        if let Err(e) = tx_for_cargo
                            .send(CargoDocumentEvent::CargoResolved(output))
                            .await
                        {
                            error!("error sending cargo resolved event: {}", e);
                        }
                    }
                    Err(err) => {
                        error!("error resolving: {}", err);
                        if let Err(e) = tx_for_cargo
                            .send(CargoDocumentEvent::CargoDiagnostic(event.uri.clone(), err))
                            .await
                        {
                            error!("error sending diagnostic event: {}", e);
                        }
                    }
                }
            }
        });

        // Timer task
        let mut debouncer = Debouncer::new(tx.clone(), 1000, 5000);
        debouncer.spawn();

        // Audit task
        let mut audit_controller = AuditController::new(tx.clone());
        audit_controller.spawn();

        // Main loop
        let render_tx = self.render_tx.clone();
        let client = self.client.clone();
        let client_capabilities = self.client_capabilities.clone();
        let cargo_path = self.cargo_path.clone();

        // Shared HTTP client for crates.io API requests
        let http_client = reqwest::Client::builder()
            .user_agent("lsp-cargo-appraiser")
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        tokio::spawn(async move {
            // Workspace state
            let mut state = Workspace::new();

            // Diagnostic controller
            let diag_client = client.clone();
            let mut diagnostic_controller = DiagnosticController::new(diag_client);

            while let Some(event) = rx.recv().await {
                // Build context for handlers
                let mut ctx = AppraiserContext {
                    state: &mut state,
                    diagnostic_controller: &mut diagnostic_controller,
                    render_tx: &render_tx,
                    debouncer: &debouncer,
                    audit_controller: &audit_controller,
                    cargo_tx: &cargo_tx,
                    inner_tx: &inner_tx,
                    client: &client,
                    client_capabilities: &client_capabilities,
                    http_client: &http_client,
                    cargo_path: &cargo_path,
                };

                match event {
                    CargoDocumentEvent::Audited(reports) => {
                        handle_audited(&mut ctx, reports).await;
                    }
                    CargoDocumentEvent::CargoDiagnostic(uri, err) => {
                        handle_cargo_diagnostic(&mut ctx, uri, err).await;
                    }
                    CargoDocumentEvent::Hovered(uri, pos, tx) => {
                        handle_hover(&mut ctx, uri, pos, tx).await;
                    }
                    CargoDocumentEvent::Gded(uri, pos, tx) => {
                        handle_gd(&mut ctx, uri, pos, tx).await;
                    }
                    CargoDocumentEvent::Completion(uri, pos, tx) => {
                        handle_completion(&mut ctx, uri, pos, tx).await;
                    }
                    CargoDocumentEvent::CodeAction(uri, range, tx) => {
                        handle_code_action(&mut ctx, uri, range, tx).await;
                    }
                    CargoDocumentEvent::Closed(uri) => {
                        handle_closed(&mut ctx, uri).await;
                    }
                    CargoDocumentEvent::CargoLockChanged => {
                        handle_cargo_lock_changed(&mut ctx).await;
                    }
                    CargoDocumentEvent::Changed(msg) => {
                        handle_changed(&mut ctx, msg).await;
                    }
                    CargoDocumentEvent::Parse(uri) => {
                        handle_parse(&mut ctx, uri).await;
                    }
                    CargoDocumentEvent::Opened(msg) | CargoDocumentEvent::Saved(msg) => {
                        handle_opened_saved(&mut ctx, msg).await;
                    }
                    CargoDocumentEvent::ReadyToResolve(event_ctx) => {
                        handle_ready_to_resolve(&mut ctx, event_ctx).await;
                    }
                    CargoDocumentEvent::CargoResolved(output) => {
                        handle_cargo_resolved(&mut ctx, output).await;
                    }
                }
            }
        });

        tx
    }
}
