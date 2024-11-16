use std::collections::{hash_map::Entry, HashMap};

use tower_lsp::lsp_types::Uri;
use tracing::info;

use crate::entity::{EntryDiff, TomlParsingError};

use super::document::Document;

pub struct Workspace {
    pub documents: HashMap<Uri, Document>,
}

impl Workspace {
    pub fn new() -> Self {
        Self {
            documents: HashMap::new(),
        }
    }

    pub fn document(&self, uri: &Uri) -> Option<&Document> {
        self.documents.get(uri)
    }

    pub fn check_rev(&self, uri: &Uri, rev: usize) -> bool {
        self.document(uri)
            .map(|doc| doc.rev == rev)
            .unwrap_or(false)
    }

    pub fn document_mut(&mut self, uri: &Uri) -> Option<&mut Document> {
        self.documents.get_mut(uri)
    }

    pub fn document_mut_with_rev(&mut self, uri: &Uri, rev: usize) -> Option<&mut Document> {
        self.documents
            .get_mut(uri)
            .and_then(|doc| if doc.rev != rev { None } else { Some(doc) })
    }

    pub fn del(&mut self, uri: &Uri) {
        self.documents.remove(uri);
    }

    pub fn mark_all_dirty(&mut self) -> Vec<(Uri, usize)> {
        let mut uris = Vec::new();
        for doc in self.documents.values_mut() {
            doc.mark_dirty();
            uris.push((doc.uri.clone(), doc.rev));
        }
        uris
    }

    pub fn reconsile(
        &mut self,
        uri: &Uri,
        text: &str,
    ) -> Result<(&Document, EntryDiff), Vec<TomlParsingError>> {
        let mut new_doc = Document::parse(uri, text);
        if !new_doc.parsing_errors.is_empty() {
            return Err(new_doc.parsing_errors);
        }
        match self.documents.entry(uri.clone()) {
            Entry::Occupied(entry) => {
                let diff = Document::diff(Some(entry.get()), &new_doc);
                let doc = entry.into_mut();
                if !diff.is_empty() {
                    doc.reconsile(new_doc, &diff);
                    doc.populate_dependencies();
                }
                Ok((doc, diff))
            }
            Entry::Vacant(entry) => {
                let diff = Document::diff(None, &new_doc);
                new_doc.self_reconsile(&diff);
                new_doc.populate_dependencies();
                let doc = entry.insert(new_doc);
                Ok((doc, diff))
            }
        }
    }

    pub fn populate_dependencies(&mut self, uri: &Uri) {
        if let Some(doc) = self.document_mut(uri) {
            doc.populate_dependencies();
        }
    }
}
