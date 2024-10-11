use std::collections::{hash_map::Entry, HashMap};

use cargo::ops::new;
use tower_lsp::lsp_types::Url;

use super::{document::Document, symbol_tree::SymbolDiff};

pub struct Workspace {
    pub cur_uri: Option<Url>,
    pub documents: HashMap<Url, Document>,
}

impl Workspace {
    pub fn new() -> Self {
        Self {
            cur_uri: None,
            documents: HashMap::new(),
        }
    }

    pub fn state(&self, uri: &Url) -> Option<&Document> {
        self.documents.get(uri)
    }

    pub fn state_mut(&mut self, uri: &Url) -> Option<&mut Document> {
        self.documents.get_mut(uri)
    }

    pub fn state_mut_with_rev(&mut self, uri: &Url, rev: usize) -> Option<&mut Document> {
        self.documents
            .get_mut(uri)
            .and_then(|doc| if doc.rev != rev { None } else { Some(doc) })
    }

    pub fn del(&mut self, uri: &Url) {
        self.documents.remove(uri);
    }

    pub fn clear_except_current(&mut self) -> Option<&Document> {
        match self.cur_uri.as_ref() {
            None => None,
            Some(uri) => {
                self.documents.retain(|_, doc| &doc.uri == uri);
                self.state(uri)
            }
        }
    }

    pub fn partial_reconsile(&mut self, uri: &Url, text: &str) -> SymbolDiff {
        let mut new_doc = Document::parse(uri, text);
        self.cur_uri = Some(uri.clone());
        //TODO refactor, maybe we can do better to avoid string allocation
        match self.documents.entry(uri.clone()) {
            Entry::Occupied(mut entry) => {
                let diff = Document::diff_symbols(Some(entry.get()), &new_doc);
                entry.get_mut().reconsile(new_doc, &diff);
                diff
            }
            Entry::Vacant(entry) => {
                let diff = Document::diff_symbols(None, &new_doc);
                new_doc.self_reconsile(&diff);
                entry.insert(new_doc);
                diff
            }
        }
    }

    pub fn reconsile(&mut self, uri: &Url, text: &str) -> SymbolDiff {
        let mut new_doc = Document::parse(uri, text);
        self.cur_uri = Some(uri.clone());
        match self.documents.entry(uri.clone()) {
            Entry::Occupied(mut entry) => {
                let diff = Document::diff_symbols(Some(entry.get()), &new_doc);
                entry.get_mut().reconsile(new_doc, &diff);
                entry.get_mut().populate_dependencies();
                diff
            }
            Entry::Vacant(entry) => {
                let diff = Document::diff_symbols(None, &new_doc);
                new_doc.self_reconsile(&diff);
                new_doc.populate_dependencies();
                entry.insert(new_doc);
                diff
            }
        }
    }
}
