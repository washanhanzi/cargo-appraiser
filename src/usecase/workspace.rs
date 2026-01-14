use std::collections::HashMap;

use tower_lsp::lsp_types::Uri;

use crate::entity::CanonicalUri;

use super::document::Document;

pub struct Workspace {
    pub documents: HashMap<CanonicalUri, Document>,
    /// Workspace member package names (for audit)
    pub member_names: Vec<String>,
    pub root_manifest_uri: Option<CanonicalUri>,
    pub member_manifest_uris: Vec<CanonicalUri>,
    pub uris: HashMap<CanonicalUri, Uri>,
}

impl Workspace {
    pub fn new() -> Self {
        Self {
            documents: HashMap::new(),
            member_names: Vec::new(),
            root_manifest_uri: None,
            member_manifest_uris: Vec::new(),
            uris: HashMap::new(),
        }
    }

    pub fn document(&self, uri: &CanonicalUri) -> Option<&Document> {
        self.documents.get(uri)
    }

    pub fn uri(&self, uri: &CanonicalUri) -> Option<&Uri> {
        self.uris.get(uri)
    }

    pub fn root_document(&self) -> Option<&Document> {
        self.root_manifest_uri
            .as_ref()
            .and_then(|uri| self.documents.get(uri))
    }

    pub fn check_rev(&self, uri: &CanonicalUri, rev: usize) -> bool {
        self.document(uri)
            .map(|doc| doc.rev == rev)
            .unwrap_or(false)
    }

    pub fn document_mut_with_rev(
        &mut self,
        uri: &CanonicalUri,
        rev: usize,
    ) -> Option<&mut Document> {
        self.documents
            .get_mut(uri)
            .and_then(|doc| if doc.rev != rev { None } else { Some(doc) })
    }

    pub fn mark_all_dirty(&mut self) -> Vec<(CanonicalUri, usize)> {
        let mut uris = Vec::new();
        for doc in self.documents.values_mut() {
            doc.mark_dirty();
            uris.push((doc.canonical_uri.clone(), doc.rev));
        }
        uris
    }

    /// Remove a document from the workspace.
    /// Clears workspace-level state when all documents are closed.
    pub fn remove(&mut self, uri: &CanonicalUri) {
        self.documents.remove(uri);
        self.uris.remove(uri);

        // Clear workspace-level state when all documents are closed
        if self.documents.is_empty() {
            self.member_names.clear();
            self.root_manifest_uri = None;
            self.member_manifest_uris.clear();
        }
    }

    /// Parse and store a document.
    /// Returns Ok with the document reference and any parsing errors.
    /// The document will have all dependencies marked as dirty.
    pub fn update(
        &mut self,
        uri: Uri,
        canonical_uri: CanonicalUri,
        text: &str,
    ) -> (&Document, Vec<toml_parser::ParseError>) {
        // Get next rev from existing document, or start at 1
        let next_rev = self
            .documents
            .get(&canonical_uri)
            .map(|d| d.rev + 1)
            .unwrap_or(1);

        let mut new_doc = Document::parse(uri.clone(), canonical_uri.clone(), text);
        let errors = new_doc.parsing_errors.clone();

        // Set rev and mark all dependencies as dirty
        new_doc.rev = next_rev;
        for id in new_doc.dependency_ids().cloned().collect::<Vec<_>>() {
            new_doc.dirty_dependencies.insert(id, next_rev);
        }

        // Preserve all resolved data from old document
        // Stale entries are harmless and will be updated on next CargoResolved event
        if let Some(old_doc) = self.documents.get(&canonical_uri) {
            new_doc.resolved = old_doc.resolved.clone();
        }

        self.uris.insert(canonical_uri.clone(), uri);
        self.documents.insert(canonical_uri.clone(), new_doc);
        (self.documents.get(&canonical_uri).unwrap(), errors)
    }
}
