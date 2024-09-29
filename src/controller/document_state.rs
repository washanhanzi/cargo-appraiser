use std::collections::HashMap;

use crate::{
    entity::{CargoNode, Dependency},
    usecase::ReverseSymbolTree,
};

pub struct DocumentState {
    pub possible_cur: Option<String>,
    pub rev_map: HashMap<String, usize>,
    pub symbol_map: HashMap<String, HashMap<String, CargoNode>>,
    pub reverse_map: HashMap<String, ReverseSymbolTree>,
    pub dependencies: HashMap<String, Vec<Dependency>>,
    pub dirty_nodes: HashMap<String, HashMap<String, usize>>,
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
        path: &str,
    ) -> Option<(
        &HashMap<String, CargoNode>,
        &ReverseSymbolTree,
        &Vec<Dependency>,
    )> {
        let Some(symbol_map) = self.symbol_map(path) else {
            return None;
        };
        let Some(reverse_map) = self.reverse_map(path) else {
            return None;
        };
        let Some(dependencies) = self.dependencies(path) else {
            return None;
        };
        Some((symbol_map, reverse_map, dependencies))
    }

    pub fn state_mut(
        &mut self,
        path: &str,
    ) -> (
        &mut HashMap<String, CargoNode>,
        &mut ReverseSymbolTree,
        &mut HashMap<String, usize>,
        &mut Vec<Dependency>,
    ) {
        let symbol_map = self
            .symbol_map
            .entry(path.to_string())
            .or_insert_with(HashMap::new);
        let reverse_map = self
            .reverse_map
            .entry(path.to_string())
            .or_insert_with(|| ReverseSymbolTree::new());
        let dependencies = self
            .dependencies
            .entry(path.to_string())
            .or_insert_with(Vec::new);
        let dirty_nodes = self
            .dirty_nodes
            .entry(path.to_string())
            .or_insert_with(HashMap::new);
        (symbol_map, reverse_map, dirty_nodes, dependencies)
    }

    pub fn check(&self, path: &str, rev: usize) -> bool {
        self.rev(path) == rev
    }

    pub fn active(&mut self, path: &str) {
        self.possible_cur = Some(path.to_string());
    }

    pub fn symbol_map(&self, path: &str) -> Option<&HashMap<String, CargoNode>> {
        self.symbol_map.get(path)
    }
    pub fn symbol_map_mut(&mut self, path: &str) -> Option<&mut HashMap<String, CargoNode>> {
        self.symbol_map.get_mut(path)
    }

    pub fn reverse_map(&self, path: &str) -> Option<&ReverseSymbolTree> {
        self.reverse_map.get(path)
    }

    pub fn reverse_map_mut(&mut self, path: &str) -> Option<&mut ReverseSymbolTree> {
        self.reverse_map.get_mut(path)
    }

    pub fn dependencies(&self, path: &str) -> Option<&Vec<Dependency>> {
        self.dependencies.get(path)
    }

    pub fn dependencies_mut(&mut self, path: &str) -> Option<&mut Vec<Dependency>> {
        self.dependencies.get_mut(path)
    }

    pub fn dirty_nodes(&self, path: &str) -> Option<&HashMap<String, usize>> {
        self.dirty_nodes.get(path)
    }

    pub fn dirty_nodes_mut(&mut self, path: &str) -> Option<&mut HashMap<String, usize>> {
        self.dirty_nodes.get_mut(path)
    }

    pub fn is_empty(&self) -> bool {
        self.rev_map.is_empty()
    }

    pub fn inc_rev(&mut self, path: &str) -> usize {
        *self.rev_map.entry(path.to_string()).or_insert(0) += 1;
        self.rev(path)
    }

    pub fn rev(&self, path: &str) -> usize {
        *self.rev_map.get(path).unwrap_or(&0)
    }

    pub fn close(&mut self, path: &str) {
        self.rev_map.remove(path);
        self.symbol_map.remove(path);
        self.reverse_map.remove(path);
        self.dependencies.remove(path);
        self.dirty_nodes.remove(path);
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
