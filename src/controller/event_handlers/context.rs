//! Shared context for event handlers.

use tokio::sync::mpsc::Sender;
use tower_lsp::Client;

use crate::{decoration::DecorationEvent, usecase::Workspace};

use super::super::{
    appraiser::{CargoDocumentEvent, Ctx},
    audit::AuditController,
    capabilities::ClientCapabilities,
    debouncer::Debouncer,
    diagnostic::DiagnosticController,
};

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
