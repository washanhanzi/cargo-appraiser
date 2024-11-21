use std::collections::HashMap;

use tower_lsp::{
    lsp_types::{Diagnostic, Uri},
    Client,
};
use tracing::Instrument;

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
            diags_map.retain(|k, _| !matches!(k.kind, DiagnosticKind::Audit));

            // Update diagnostics display
            let diags: Vec<Diagnostic> = diags_map.values().cloned().collect();
            publish(&self.client, uri, diags).await;
        }
    }
}

async fn publish(client: &Client, uri: &Uri, diags: Vec<Diagnostic>) {
    if !diags.is_empty() {
        client.publish_diagnostics(uri.clone(), diags, None).await
    }
}
