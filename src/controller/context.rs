//! Shared context and types for the Appraiser controller.

use tokio::sync::{mpsc::Sender, oneshot};
use tower_lsp::{
    lsp_types::{
        CodeActionResponse, CompletionResponse, GotoDefinitionResponse, Hover, Position, Range, Uri,
    },
    Client,
};

use crate::{
    decoration::DecorationEvent,
    entity::{CanonicalUri, CargoError},
    usecase::Workspace,
};

use super::{
    audit::AuditController,
    capabilities::ClientCapabilities,
    cargo::CargoResolveOutput,
    debouncer::Debouncer,
    diagnostic::DiagnosticController,
};

/// Context for cargo resolution tasks.
#[derive(Debug, Clone)]
pub struct Ctx {
    pub uri: CanonicalUri,
    pub rev: usize,
}

/// Events that can be sent to the Appraiser controller.
pub enum CargoDocumentEvent {
    Opened(CargoTomlPayload),
    Saved(CargoTomlPayload),
    /// Parse event won't trigger Cargo.toml resolve compared to Opened and Saved
    Parse(Uri),
    Changed(CargoTomlPayload),
    ReadyToResolve(Ctx),
    /// Mark dependencies dirty, clear decorations
    Closed(Uri),
    /// Result from cargo
    CargoResolved(CargoResolveOutput),
    /// Cargo.lock change
    CargoLockChanged,
    /// Code action request
    CodeAction(Uri, Range, oneshot::Sender<CodeActionResponse>),
    /// Hover event
    Hovered(Uri, Position, oneshot::Sender<Option<Hover>>),
    /// Completion request
    Completion(Uri, Position, oneshot::Sender<Option<CompletionResponse>>),
    /// Goto definition request
    Gded(
        Uri,
        Position,
        oneshot::Sender<Option<GotoDefinitionResponse>>,
    ),
    /// Cargo diagnostic error
    CargoDiagnostic(CanonicalUri, CargoError),
    /// Audit results
    Audited(super::audit::AuditReports),
}

/// Payload for Cargo.toml document events.
pub struct CargoTomlPayload {
    pub uri: Uri,
    pub text: String,
}

/// Shared context passed to all event handlers.
///
/// This struct holds references to all the shared state and services
/// needed by event handlers, avoiding the need to pass many parameters
/// to each handler function.
pub struct AppraiserContext<'a> {
    /// Workspace state containing all open documents
    pub state: &'a mut Workspace,
    /// Diagnostic controller for managing LSP diagnostics
    pub diagnostic_controller: &'a mut DiagnosticController,
    /// Sender for decoration render events
    pub render_tx: &'a Sender<DecorationEvent>,
    /// Debouncer for rate-limiting cargo resolve requests
    pub debouncer: &'a Debouncer,
    /// Audit controller for security audits
    pub audit_controller: &'a AuditController,
    /// Sender for cargo resolve tasks
    pub cargo_tx: &'a Sender<Ctx>,
    /// Sender for internal event loop messages
    pub inner_tx: &'a Sender<CargoDocumentEvent>,
    /// LSP client for sending requests/notifications
    pub client: &'a Client,
    /// Client capabilities detected during initialization
    pub client_capabilities: &'a ClientCapabilities,
    /// Shared HTTP client for crates.io requests
    pub http_client: &'a reqwest::Client,
    /// Path to the cargo executable
    pub cargo_path: &'a str,
}
