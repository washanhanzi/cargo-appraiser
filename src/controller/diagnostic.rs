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
    pub cargo_diagnostics: HashMap<Uri, HashMap<String, Diagnostic>>,
    pub parse_diagnostics: HashMap<Uri, HashMap<String, Diagnostic>>,
    rev: HashMap<Uri, i32>,
}

impl DiagnosticController {
    pub fn new(client: Client) -> Self {
        DiagnosticController {
            client,
            cargo_diagnostics: HashMap::new(),
            parse_diagnostics: HashMap::new(),
            rev: HashMap::new(),
        }
    }

    pub async fn add_cargo_diagnostic(&mut self, uri: &Uri, id: &str, diag: Diagnostic) {
        self.cargo_diagnostics
            .entry(uri.clone())
            .or_default()
            .insert(id.to_string(), diag);
        let diags_map = self.cargo_diagnostics.get(uri).unwrap();
        // Update the revision number for the given URI
        let rev = self.rev.entry(uri.clone()).or_insert(0);
        *rev += 1;
        let diags: Vec<Diagnostic> = diags_map.values().cloned().collect();
        match self.parse_diagnostics.get(uri) {
            Some(parse_diags_map) => {
                let parse_diags: Vec<Diagnostic> = parse_diags_map.values().cloned().collect();
                let all_diags = [parse_diags, diags].concat();
                //send diagnostics
                self.client
                    .publish_diagnostics(uri.clone(), all_diags, Some(*rev))
                    .await;
            }
            None => {
                self.client
                    .publish_diagnostics(uri.clone(), diags, Some(*rev))
                    .await;
            }
        }
    }

    pub async fn add_parse_diagnostic(&mut self, uri: &Uri, id: &str, diag: Diagnostic) {
        self.parse_diagnostics
            .entry(uri.clone())
            .or_default()
            .insert(id.to_string(), diag);
        let diags_map = self.parse_diagnostics.get(uri).unwrap();
        // Update the revision number for the given URI
        let diags: Vec<Diagnostic> = diags_map.values().cloned().collect();
        match self.cargo_diagnostics.get(uri) {
            Some(cargo_diags_map) => {
                let cargo_diags: Vec<Diagnostic> = cargo_diags_map.values().cloned().collect();
                let all_diags = [cargo_diags, diags].concat();
                self.client
                    .publish_diagnostics(uri.clone(), all_diags, None)
                    .await;
            }
            None => {
                self.client
                    .publish_diagnostics(uri.clone(), diags, None)
                    .await;
            }
        }
    }

    pub async fn clear_cargo_diagnostics(&mut self, uri: &Uri) {
        self.cargo_diagnostics.remove(uri);
        self.rev.remove(uri);
        if self.parse_diagnostics.is_empty() {
            self.client
                .publish_diagnostics(uri.clone(), vec![], None)
                .await
        } else {
            let diags_map = self.parse_diagnostics.get(uri).unwrap();
            let diags: Vec<Diagnostic> = diags_map.values().cloned().collect();
            self.client
                .publish_diagnostics(uri.clone(), diags, None)
                .await
        }
    }

    pub async fn clear_parse_diagnostics(&mut self, uri: &Uri) {
        self.parse_diagnostics.remove(uri);
        if self.cargo_diagnostics.is_empty() {
            self.client
                .publish_diagnostics(uri.clone(), vec![], None)
                .await
        } else {
            let diags_map = self.cargo_diagnostics.get(uri).unwrap();
            let diags: Vec<Diagnostic> = diags_map.values().cloned().collect();
            self.client
                .publish_diagnostics(uri.clone(), diags, None)
                .await
        }
    }
}
