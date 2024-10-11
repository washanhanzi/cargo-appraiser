use std::collections::HashMap;

use tower_lsp::lsp_types::Position;

use crate::entity::CargoNode;

#[derive(Debug, Clone)]
pub struct ReverseSymbolTree(HashMap<u32, Vec<String>>);

impl ReverseSymbolTree {
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    pub fn parse(symbols: &HashMap<String, CargoNode>) -> Self {
        let mut m: HashMap<u32, Vec<String>> = HashMap::new();
        for (id, node) in symbols {
            for line in node.range.start.line..=node.range.end.line {
                m.entry(line).or_default().push(id.clone());
            }
        }
        Self(m)
    }

    pub fn init(&mut self, symbols: &HashMap<String, CargoNode>) {
        if symbols.is_empty() {
            return;
        }
        let mut m: HashMap<u32, Vec<String>> = HashMap::new();
        for (id, node) in symbols {
            for line in node.range.start.line..=node.range.end.line {
                m.entry(line).or_default().push(id.clone());
            }
        }
        *self = Self(m);
    }

    //TODO for simple or table dependency, the crate name will never be matched
    //if no match found, return the simple or table dependency node?
    pub fn precise_match(
        &self,
        pos: Position,
        symbol_map: &HashMap<String, CargoNode>,
    ) -> Option<CargoNode> {
        let ids = self.0.get(&pos.line)?;
        let mut best_match: Option<CargoNode> = None;
        let mut best_width: u32 = u32::MAX;

        for id in ids {
            let Some(node) = symbol_map.get(id) else {
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
