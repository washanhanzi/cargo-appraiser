use std::collections::HashMap;

use tower_lsp::{
    lsp_types::{Diagnostic, Url},
    Client,
};

use crate::entity::{CargoError, Dependency, TomlKey};

pub struct DiagnosticController {
    client: Client,
    pub diagnostics: HashMap<Url, HashMap<String, Diagnostic>>,
    rev: HashMap<Url, i32>,
}

impl DiagnosticController {
    pub fn new(client: Client) -> Self {
        DiagnosticController {
            client,
            diagnostics: HashMap::new(),
            rev: HashMap::new(),
        }
    }

    pub async fn add(&mut self, uri: &Url, id: &str, diag: Diagnostic) {
        // self.diagnostics.entry(uri).or_default().push(diagnostic);
        self.diagnostics
            .entry(uri.clone())
            .or_default()
            .insert(id.to_string(), diag);
        let diags_map = self.diagnostics.get(uri).unwrap();
        // Update the revision number for the given URI
        let rev = self.rev.entry(uri.clone()).or_insert(0);
        *rev += 1;
        let diags: Vec<Diagnostic> = diags_map.values().cloned().collect();
        //send diagnostics
        self.client
            .publish_diagnostics(uri.clone(), diags, Some(*rev))
            .await;
    }

    pub async fn clear(&mut self, uri: &Url) {
        self.diagnostics.remove(uri);
        self.client
            .publish_diagnostics(uri.clone(), vec![], None)
            .await
    }
}
