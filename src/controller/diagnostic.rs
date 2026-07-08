use std::collections::HashMap;

use tower_lsp::{
    lsp_types::{Diagnostic, Uri},
    Client,
};
use tracing::debug;

use crate::entity::{CanonicalUri, CargoError, TomlDependency, TomlNode};

use super::context::{AppraiserContext, CargoDocumentEvent};

/// Handle `CargoDocumentEvent::CargoDiagnostic` - process cargo errors as diagnostics.
pub async fn handle_cargo_diagnostic(
    ctx: &mut AppraiserContext<'_>,
    uri: CanonicalUri,
    err: CargoError,
) {
    debug!(
        "Appraiser Event: CargoDiagnostic for URI: {:?}, Error: {:?}",
        uri, err
    );

    let Some(client_uri) = ctx.state.uri(&uri) else {
        return;
    };

    ctx.diagnostic_controller
        .clear_cargo_diagnostics(client_uri)
        .await;

    // We need a crate name to find something in toml
    let Some(crate_name) = err.crate_name() else {
        return;
    };

    let Some(doc) = ctx.state.document(&uri) else {
        return;
    };

    let keys: Vec<TomlNode> = doc
        .find_keys_by_crate_name(crate_name)
        .into_iter()
        .cloned()
        .collect();
    let deps: Vec<TomlDependency> = doc
        .find_deps_by_crate_name(crate_name)
        .into_iter()
        .cloned()
        .collect();
    let tree = doc.tree().clone();
    let client_uri = client_uri.clone();
    let inner_tx = ctx.inner_tx.clone();

    // Computing diagnostics may query the cargo registry (network/disk I/O),
    // so run it off the event loop and post the result back as an event.
    tokio::spawn(async move {
        let digs = tokio::task::spawn_blocking(move || {
            let key_refs: Vec<&TomlNode> = keys.iter().collect();
            let dep_refs: Vec<&TomlDependency> = deps.iter().collect();
            err.diagnostic(&key_refs, &dep_refs, &tree)
        })
        .await
        .ok()
        .flatten();

        if let Some(digs) = digs {
            if let Err(e) = inner_tx.send(CargoDocumentEvent::CargoDiagnosticsComputed(
                client_uri, digs,
            )) {
                debug!("failed to send computed cargo diagnostics: {}", e);
            }
        }
    });
}

//we need to distinguish between parsing erros and cargo error
//parsing errors can be cleared on file change
//cargo errors can be only cleared on success cargo resolve
pub struct DiagnosticController {
    client: Client,
    diagnostics: HashMap<Uri, HashMap<DiagnosticKey, Diagnostic>>,
    rev: HashMap<Uri, i32>,
}

#[derive(Hash, Eq, PartialEq)]
struct DiagnosticKey {
    id: String,
    kind: DiagnosticKind,
}

#[derive(Hash, Eq, PartialEq)]
enum DiagnosticKind {
    Cargo,
    Parse,
    Audit,
}

impl DiagnosticController {
    pub fn new(client: Client) -> Self {
        DiagnosticController {
            client,
            diagnostics: HashMap::new(),
            rev: HashMap::new(),
        }
    }

    pub async fn add_cargo_diagnostic(&mut self, uri: &Uri, id: &str, diag: Diagnostic) {
        self.diagnostics.entry(uri.clone()).or_default().insert(
            DiagnosticKey {
                id: id.to_string(),
                kind: DiagnosticKind::Cargo,
            },
            diag,
        );
        let diags_map = self.diagnostics.get(uri).unwrap();
        // Update the revision number for the given URI
        let rev = self.rev.entry(uri.clone()).or_insert(0);
        *rev += 1;
        let diags: Vec<Diagnostic> = diags_map.values().cloned().collect();
        publish(&self.client, uri, diags).await;
    }

    pub async fn add_parse_diagnostic(&mut self, uri: &Uri, id: &str, diag: Diagnostic) {
        self.diagnostics.entry(uri.clone()).or_default().insert(
            DiagnosticKey {
                id: id.to_string(),
                kind: DiagnosticKind::Parse,
            },
            diag,
        );
        let diags_map = self.diagnostics.get(uri).unwrap();
        // Update the revision number for the given URI
        let diags: Vec<Diagnostic> = diags_map.values().cloned().collect();
        publish(&self.client, uri, diags).await;
    }

    pub async fn clear_cargo_diagnostics(&mut self, uri: &Uri) {
        if let Some(diags_map) = self.diagnostics.get_mut(uri) {
            diags_map.retain(|k, _| !matches!(k.kind, DiagnosticKind::Cargo));

            // Update diagnostics display
            let diags: Vec<Diagnostic> = diags_map.values().cloned().collect();
            publish(&self.client, uri, diags).await;
        }
    }

    pub async fn clear_parse_diagnostics(&mut self, uri: &Uri) {
        if let Some(diags_map) = self.diagnostics.get_mut(uri) {
            diags_map.retain(|k, _| !matches!(k.kind, DiagnosticKind::Parse));

            // Update diagnostics display
            let diags: Vec<Diagnostic> = diags_map.values().cloned().collect();
            publish(&self.client, uri, diags).await;
        }
    }

    pub async fn add_audit_diagnostic(&mut self, uri: &Uri, id: &str, diag: Diagnostic) {
        self.diagnostics.entry(uri.clone()).or_default().insert(
            DiagnosticKey {
                id: id.to_string(),
                kind: DiagnosticKind::Audit,
            },
            diag,
        );
        let diags_map = self.diagnostics.get(uri).unwrap();
        // Update the revision number for the given URI
        let diags: Vec<Diagnostic> = diags_map.values().cloned().collect();
        publish(&self.client, uri, diags).await;
    }

    pub async fn clear_audit_diagnostics(&mut self) {
        for (uri, diags_map) in self.diagnostics.iter_mut() {
            //retain Parse and Cargo kind
            diags_map
                .retain(|k, _| matches!(k.kind, DiagnosticKind::Parse | DiagnosticKind::Cargo));

            // Update diagnostics display
            let diags: Vec<Diagnostic> = diags_map.values().cloned().collect();
            publish(&self.client, uri, diags).await;
        }
    }
}

async fn publish(client: &Client, uri: &Uri, diags: Vec<Diagnostic>) {
    client.publish_diagnostics(uri.clone(), diags, None).await
}
