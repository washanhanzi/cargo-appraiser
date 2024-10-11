use std::collections::HashMap;

use tower_lsp::lsp_types::{Position, Url};

use crate::entity::{cargo_dependency_to_toml_key, CargoNode, Dependency};

use super::{
    diff_symbols,
    symbol_tree::{self, SymbolDiff},
    ReverseSymbolTree, Walker,
};

#[derive(Debug, Clone)]
pub struct Document {
    pub uri: Url,
    pub rev: usize,
    pub symbols: HashMap<String, CargoNode>,
    pub reverse_symbols: ReverseSymbolTree,
    //might be empty
    pub dependencies: HashMap<String, Dependency>,
    pub dirty_nodes: HashMap<String, usize>,
}

impl Document {
    pub fn parse(uri: &Url, text: &str) -> Self {
        let p = taplo::parser::parse(text);
        if !p.errors.is_empty() {
            //TODO
            unreachable!()
        }
        let dom = p.into_dom();
        if dom.validate().is_err() {
            //TODO
            unreachable!()
        }
        let table = dom.as_table().unwrap();
        let entries = table.entries().read();

        let mut walker = Walker::new(text, entries.len());

        for (key, entry) in entries.iter() {
            if key.value().is_empty() {
                continue;
            }
            walker.walk_root(key.value(), key.value(), entry)
        }

        let (symbols, deps) = walker.consume();
        let len = symbols.len();
        let reverse_symbols = ReverseSymbolTree::parse(&symbols);
        Self {
            uri: uri.clone(),
            rev: 0,
            symbols,
            reverse_symbols,
            dependencies: deps,
            dirty_nodes: HashMap::with_capacity(len),
        }
    }

    pub fn diff_symbols(old: Option<&Document>, new: &Document) -> SymbolDiff {
        let old = old.map(|d| &d.symbols);
        diff_symbols(old, &new.symbols)
    }

    pub fn reconsile(&mut self, mut new: Document, diff: &SymbolDiff) {
        self.symbols = new.symbols;
        self.reverse_symbols = new.reverse_symbols;
        self.rev += 1;
        //merge dependencies
        for v in diff.created.iter().chain(&diff.value_updated) {
            self.dirty_nodes.insert(v.to_string(), self.rev);
            self.dependencies
                .insert(v.to_string(), new.dependencies.get(v).unwrap().clone());
        }
        for v in &diff.range_updated {
            let dep = new.dependencies.remove(v).unwrap();
            self.dependencies.get_mut(v).unwrap().merge_range(dep);
        }
        for v in &diff.deleted {
            self.dirty_nodes.remove(v);
        }
    }

    pub fn self_reconsile(&mut self, diff: &SymbolDiff) {
        self.rev += 1;
        //merge dependencies
        for v in diff
            .created
            .iter()
            .chain(&diff.range_updated)
            .chain(&diff.value_updated)
        {
            self.dirty_nodes.insert(v.to_string(), self.rev);
        }
    }

    //TODO return error
    pub fn populate_dependencies(&mut self) {
        if let Ok(path) = self.uri.to_file_path() {
            //get dependencies
            let gctx = cargo::util::context::GlobalContext::default().unwrap();
            //TODO ERROR parse manifest
            let workspace = cargo::core::Workspace::new(path.as_path(), &gctx).unwrap();
            //TODO if it's error, it's a virtual workspace
            let current = workspace.current().unwrap();
            let mut unresolved = HashMap::with_capacity(current.dependencies().len());
            for dep in current.dependencies() {
                let key = cargo_dependency_to_toml_key(dep);
                unresolved.insert(key, dep);
            }

            for dep in self.dependencies.values_mut() {
                let key = dep.toml_key();
                if let Some(u) = unresolved.remove(&key) {
                    //update value to dep.unresolved
                    dep.unresolved = Some(u.clone());
                }
            }
        }
    }

    pub fn is_dirty(&self) -> bool {
        !self.dirty_nodes.is_empty()
    }

    pub fn precise_match(&self, pos: Position) -> Option<CargoNode> {
        self.reverse_symbols.precise_match(pos, &self.symbols)
    }

    pub fn dependency(&self, id: &str) -> Option<&Dependency> {
        self.dependencies.get(id)
    }

    pub fn symbol(&self, id: &str) -> Option<&CargoNode> {
        self.symbols.get(id)
    }
}
