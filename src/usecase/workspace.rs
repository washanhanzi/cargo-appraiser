use std::collections::{hash_map::Entry, HashMap};

use cargo::core::PackageIdSpec;
use tower_lsp::lsp_types::Uri;

use crate::entity::{CanonicalUri, EntryDiff, TomlParsingError};

use super::document::Document;

pub struct Workspace {
    pub documents: HashMap<CanonicalUri, Document>,
    pub specs: Vec<PackageIdSpec>,
    pub root_manifest_uri: Option<CanonicalUri>,
    pub member_manifest_uris: Vec<CanonicalUri>,
    pub uris: HashMap<CanonicalUri, Uri>,
}

impl Workspace {
    pub fn new() -> Self {
        Self {
            documents: HashMap::new(),
            specs: Vec::new(),
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

    pub fn document_mut(&mut self, uri: &CanonicalUri) -> Option<&mut Document> {
        self.documents.get_mut(uri)
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

    pub fn reconsile(
        &mut self,
        uri: Uri,
        canonical_uri: CanonicalUri,
        text: &str,
    ) -> Result<(&Document, EntryDiff), Vec<TomlParsingError>> {
        let mut new_doc = Document::parse(uri.clone(), canonical_uri.clone(), text);
        if !new_doc.parsing_errors.is_empty() {
            return Err(new_doc.parsing_errors);
        }
        self.uris.insert(canonical_uri.clone(), uri);
        match self.documents.entry(canonical_uri) {
            Entry::Occupied(entry) => {
                let diff = Document::diff(Some(entry.get()), &new_doc);
                let doc = entry.into_mut();
                if !diff.is_empty() {
                    doc.reconsile(new_doc, &diff);
                }
                Ok((doc, diff))
            }
            Entry::Vacant(entry) => {
                let diff = Document::diff(None, &new_doc);
                new_doc.self_reconsile(&diff);
                let doc = entry.insert(new_doc);
                Ok((doc, diff))
            }
        }
    }
}
