use std::collections::HashMap;

use tower_lsp::lsp_types::Position;

use crate::entity::{SymbolTree, TomlEntry, TomlKey, TomlNode};

#[derive(Debug, Clone)]
pub struct ReverseSymbolTree {
    entries: HashMap<u32, Vec<String>>,
    keys: HashMap<u32, Vec<String>>,
}

impl ReverseSymbolTree {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            keys: HashMap::new(),
        }
    }

    pub fn parse(tree: &SymbolTree) -> Self {
        Self {
            entries: Self::parse_entries(&tree.entries),
            keys: Self::parse_keys(&tree.keys),
        }
    }

    fn parse_entries(entries: &HashMap<String, TomlNode>) -> HashMap<u32, Vec<String>> {
        let mut m: HashMap<u32, Vec<String>> = HashMap::new();
        for (id, node) in entries {
            for line in node.range.start.line..=node.range.end.line {
                m.entry(line).or_default().push(id.clone());
            }
        }
        m
    }

    fn parse_keys(keys: &HashMap<String, TomlNode>) -> HashMap<u32, Vec<String>> {
        let mut m: HashMap<u32, Vec<String>> = HashMap::new();
        for (id, node) in keys {
            for line in node.range.start.line..=node.range.end.line {
                m.entry(line).or_default().push(id.clone());
            }
        }
        m
    }

    //TODO for simple or table dependency, the crate name will never be matched
    //if no match found, return the simple or table dependency node?
    pub fn precise_match_entry(
        &self,
        pos: Position,
        entries: &HashMap<String, TomlNode>,
    ) -> Option<TomlNode> {
        let ids = self.entries.get(&pos.line)?;
        let mut best_match: Option<TomlNode> = None;
        let mut best_width: u32 = u32::MAX;

        for id in ids {
            let Some(node) = entries.get(id) else {
                continue;
            };
            if node.range.start.character <= pos.character
                && node.range.end.character >= pos.character
            {
                let width = node.range.end.character - node.range.start.character;
                if width < best_width {
                    best_width = width;
                    best_match = Some(node.clone());
                }
            }
        }

        best_match
    }

    pub fn precise_match_key(
        &self,
        pos: Position,
        keys: &HashMap<String, TomlNode>,
    ) -> Option<TomlNode> {
        let ids = self.keys.get(&pos.line)?;
        let mut best_match: Option<TomlNode> = None;
        let mut best_width: u32 = u32::MAX;

        for id in ids {
            let Some(node) = keys.get(id) else {
                continue;
            };
            if node.range.start.character <= pos.character
                && node.range.end.character >= pos.character
            {
                let width = node.range.end.character - node.range.start.character;
                if width < best_width {
                    best_width = width;
                    best_match = Some(node.clone());
                }
            }
        }

        best_match
    }
}
