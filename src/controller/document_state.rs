use std::collections::HashMap;

use tower_lsp::lsp_types::Url;

use crate::{
    entity::{CargoNode, Dependency},
    usecase::ReverseSymbolTree,
};

pub struct DocumentState {
    pub possible_cur: Option<String>,
    pub rev_map: HashMap<Url, usize>,
    pub symbol_map: HashMap<Url, HashMap<String, CargoNode>>,
    pub reverse_map: HashMap<Url, ReverseSymbolTree>,
    pub dependencies: HashMap<Url, Vec<Dependency>>,
    pub dirty_nodes: HashMap<Url, HashMap<String, usize>>,
}

impl DocumentState {
    pub fn new() -> Self {
        Self {
            possible_cur: None,
            rev_map: HashMap::new(),
            symbol_map: HashMap::new(),
            reverse_map: HashMap::new(),
            dependencies: HashMap::new(),
            dirty_nodes: HashMap::new(),
        }
    }

    pub fn state(
        &self,
        uri: &Url,
    ) -> Option<(
        &HashMap<String, CargoNode>,
        &ReverseSymbolTree,
        &Vec<Dependency>,
    )> {
        let Some(symbol_map) = self.symbol_map(uri) else {
            return None;
        };
        let Some(reverse_map) = self.reverse_map(uri) else {
            return None;
        };
        let Some(dependencies) = self.dependencies(uri) else {
            return None;
        };
        Some((symbol_map, reverse_map, dependencies))
    }

    pub fn state_mut(
        &mut self,
        uri: &Url,
    ) -> (
        &mut HashMap<String, CargoNode>,
        &mut ReverseSymbolTree,
        &mut HashMap<String, usize>,
        &mut Vec<Dependency>,
    ) {
        let symbol_map = self
            .symbol_map
            .entry(uri.clone())
            .or_insert_with(HashMap::new);
        let reverse_map = self
            .reverse_map
            .entry(uri.clone())
            .or_insert_with(|| ReverseSymbolTree::new());
        let dependencies = self
            .dependencies
            .entry(uri.clone())
            .or_insert_with(Vec::new);
        let dirty_nodes = self
            .dirty_nodes
            .entry(uri.clone())
            .or_insert_with(HashMap::new);
        (symbol_map, reverse_map, dirty_nodes, dependencies)
    }

    pub fn check(&self, uri: &Url, rev: usize) -> bool {
        self.rev(uri) == rev
    }

    pub fn active(&mut self, path: &str) {
        self.possible_cur = Some(path.to_string());
    }

    pub fn symbol_map(&self, uri: &Url) -> Option<&HashMap<String, CargoNode>> {
        self.symbol_map.get(uri)
    }
    pub fn symbol_map_mut(&mut self, uri: &Url) -> Option<&mut HashMap<String, CargoNode>> {
        self.symbol_map.get_mut(uri)
    }

    pub fn reverse_map(&self, uri: &Url) -> Option<&ReverseSymbolTree> {
        self.reverse_map.get(uri)
    }

    pub fn reverse_map_mut(&mut self, uri: &Url) -> Option<&mut ReverseSymbolTree> {
        self.reverse_map.get_mut(uri)
    }

    pub fn dependencies(&self, uri: &Url) -> Option<&Vec<Dependency>> {
        self.dependencies.get(uri)
    }

    pub fn dependencies_mut(&mut self, uri: &Url) -> Option<&mut Vec<Dependency>> {
        self.dependencies.get_mut(uri)
    }

    pub fn dirty_nodes(&self, uri: &Url) -> Option<&HashMap<String, usize>> {
        self.dirty_nodes.get(uri)
    }

    pub fn dirty_nodes_mut(&mut self, uri: &Url) -> Option<&mut HashMap<String, usize>> {
        self.dirty_nodes.get_mut(uri)
    }

    pub fn is_empty(&self) -> bool {
        self.rev_map.is_empty()
    }

    pub fn inc_rev(&mut self, uri: &Url) -> usize {
        *self.rev_map.entry(uri.clone()).or_insert(0) += 1;
        self.rev(uri)
    }

    pub fn rev(&self, uri: &Url) -> usize {
        *self.rev_map.get(uri).unwrap_or(&0)
    }

    pub fn close(&mut self, uri: &Url) {
        self.rev_map.remove(uri);
        self.symbol_map.remove(uri);
        self.reverse_map.remove(uri);
        self.dependencies.remove(uri);
        self.dirty_nodes.remove(uri);
    }

    pub fn clear(&mut self) {
        self.possible_cur = None;
        self.rev_map.clear();
        self.symbol_map.clear();
        self.reverse_map.clear();
        self.dependencies.clear();
        self.dirty_nodes.clear();
    }
}
