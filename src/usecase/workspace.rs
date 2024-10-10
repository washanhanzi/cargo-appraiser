use std::collections::HashMap;

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

    pub fn partial_reconsile(&mut self, uri: &Url, text: &str) -> SymbolDiff {
        let new_doc = Document::parse(uri, text);
        self.cur_uri = Some(uri.clone());
        //TODO refactor, maybe we can do better to avoid string allocation
        match self.state_mut(uri) {
            Some(doc) => {
                let diff = Document::diff_symbols(Some(doc), &new_doc);
                doc.partial_reconsile(new_doc);
                diff
            }
            None => {
                let diff = Document::diff_symbols(None, &new_doc);
                self.documents.insert(uri.clone(), new_doc);
                diff
            }
        }
    }

    pub fn reconsile(&mut self, uri: &Url, text: &str) -> SymbolDiff {
        let mut new_doc = Document::parse(uri, text);
        self.cur_uri = Some(uri.clone());
        match self.state_mut(uri) {
            Some(doc) => {
                let diff = Document::diff_symbols(Some(doc), &new_doc);
                doc.reconsile(new_doc, &diff);
                doc.populate_dependencies();
                diff
            }
            None => {
                let diff = Document::diff_symbols(None, &new_doc);
                new_doc.self_reconsile(&diff);
                new_doc.populate_dependencies();
                self.documents.insert(uri.clone(), new_doc);
                diff
            }
        }
    }
}
