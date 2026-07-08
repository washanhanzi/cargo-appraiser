use std::collections::HashMap;

use super::node::TomlNode;
use ls_types::Position;

/// The parsed TOML structure with efficient lookup capabilities.
///
/// Provides:
/// - O(1) key lookup by dotted ID via HashMap
/// - O(log n) position lookup via binary search on sorted vector
#[derive(Debug, Clone, Default)]
pub struct SymbolTree {
    /// O(1) key lookup by dotted ID
    nodes: HashMap<String, TomlNode>,

    /// O(log n) position lookup - IDs sorted by (start_line, start_col)
    /// Used for binary search to find nodes at a given position
    position_index: Vec<String>,

    /// prefix_max_end[i] = max end position of position_index[0..=i].
    /// Lets position lookups stop scanning backwards as soon as every
    /// remaining node is known to end before the queried position.
    prefix_max_end: Vec<(u32, u32)>,
}

impl SymbolTree {
    /// Create a new empty SymbolTree
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            position_index: Vec::new(),
            prefix_max_end: Vec::new(),
        }
    }

    /// Create a SymbolTree with pre-allocated capacity
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            nodes: HashMap::with_capacity(capacity),
            position_index: Vec::with_capacity(capacity),
            prefix_max_end: Vec::with_capacity(capacity),
        }
    }

    /// Insert a node into the tree
    pub fn insert(&mut self, node: TomlNode) {
        let id = node.id.clone();
        self.nodes.insert(id.clone(), node);
        self.position_index.push(id);
    }

    /// Build the position index after all nodes are inserted.
    /// Must be called after all insert() calls before using find_at_position().
    pub fn build_index(&mut self) {
        // Sort by (start_line, start_col)
        self.position_index.sort_by(|a, b| {
            let node_a = self.nodes.get(a).unwrap();
            let node_b = self.nodes.get(b).unwrap();
            let start_a = (node_a.range.start.line, node_a.range.start.character);
            let start_b = (node_b.range.start.line, node_b.range.start.character);
            start_a.cmp(&start_b)
        });

        self.prefix_max_end.clear();
        let mut max_end = (0, 0);
        for id in &self.position_index {
            let node = self.nodes.get(id).unwrap();
            let end = (node.range.end.line, node.range.end.character);
            max_end = max_end.max(end);
            self.prefix_max_end.push(max_end);
        }
    }

    /// Get a node by its ID (O(1) lookup)
    pub fn get(&self, id: &str) -> Option<&TomlNode> {
        self.nodes.get(id)
    }

    /// Check if the tree contains a node with the given ID
    pub fn contains(&self, id: &str) -> bool {
        self.nodes.contains_key(id)
    }

    /// Get the number of nodes in the tree
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Check if the tree is empty
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Iterate over all nodes
    pub fn iter(&self) -> impl Iterator<Item = (&String, &TomlNode)> {
        self.nodes.iter()
    }

    /// Iterate over all node IDs
    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.nodes.keys()
    }

    /// Iterate over all nodes (values only)
    pub fn values(&self) -> impl Iterator<Item = &TomlNode> {
        self.nodes.values()
    }

    /// Find the most specific (narrowest) node at the given position.
    ///
    /// Algorithm:
    /// 1. Binary search for the last node starting at or before the position
    /// 2. Scan backwards over containing nodes, stopping once the prefix max
    ///    end shows no earlier node can reach the position
    /// 3. Prefer Key nodes over Entry nodes; among same type, the narrowest
    ///
    /// Returns None if no node contains the given position.
    pub fn find_at_position(&self, pos: Position) -> Option<&TomlNode> {
        // Keys have priority over values, then narrower beats wider:
        // minimize (!is_key, width)
        let mut best: Option<((bool, u32), &TomlNode)> = None;
        self.scan_containing(pos, |node| {
            let rank = (!node.is_key(), node.width());
            if best.is_none_or(|(best_rank, _)| rank < best_rank) {
                best = Some((rank, node));
            }
        });
        best.map(|(_, node)| node)
    }

    /// Find a node at the given position, optionally filtering by key or value
    pub fn find_key_at_position(&self, pos: Position) -> Option<&TomlNode> {
        self.find_at_position_filtered(pos, |node| node.is_key())
    }

    /// Find an entry (value) node at the given position
    pub fn find_value_at_position(&self, pos: Position) -> Option<&TomlNode> {
        self.find_at_position_filtered(pos, |node| node.is_value())
    }

    /// Find the narrowest node at position matching a custom filter
    fn find_at_position_filtered<F>(&self, pos: Position, filter: F) -> Option<&TomlNode>
    where
        F: Fn(&TomlNode) -> bool,
    {
        let mut best: Option<(u32, &TomlNode)> = None;
        self.scan_containing(pos, |node| {
            if filter(node) {
                let width = node.width();
                if best.is_none_or(|(best_width, _)| width < best_width) {
                    best = Some((width, node));
                }
            }
        });
        best.map(|(_, node)| node)
    }

    /// Visit every node whose range contains `pos`.
    ///
    /// Binary-searches for the last node starting at or before `pos`, then
    /// scans backwards. The prefix max-end array bounds the scan: once every
    /// node at or before index `i` ends before `pos`, none of them (nor any
    /// earlier node) can contain it.
    fn scan_containing<'a, F>(&'a self, pos: Position, mut visit: F)
    where
        F: FnMut(&'a TomlNode),
    {
        if self.position_index.is_empty() {
            return;
        }

        let pos_tuple = (pos.line, pos.character);
        let search_idx = self.binary_search_position(pos);

        for i in (0..=search_idx).rev() {
            if self.prefix_max_end.get(i).is_some_and(|&end| end < pos_tuple) {
                break;
            }
            let Some(node) = self.nodes.get(&self.position_index[i]) else {
                continue;
            };
            let start = (node.range.start.line, node.range.start.character);
            let end = (node.range.end.line, node.range.end.character);
            if start <= pos_tuple && pos_tuple <= end {
                visit(node);
            }
        }
    }

    /// Binary search to find the index of the rightmost node where start <= pos
    fn binary_search_position(&self, pos: Position) -> usize {
        let pos_tuple = (pos.line, pos.character);

        let result = self.position_index.partition_point(|id| {
            if let Some(node) = self.nodes.get(id) {
                (node.range.start.line, node.range.start.character) <= pos_tuple
            } else {
                false
            }
        });

        // partition_point returns the first element > pos, so we want result - 1
        result.saturating_sub(1)
    }

    /// Get all nodes that are top-level dependencies
    pub fn dependencies(&self) -> impl Iterator<Item = &TomlNode> {
        self.nodes.values().filter(|n| n.kind.is_dependency())
    }
}

#[cfg(test)]
mod tests {
    use super::super::node::{DependencyKey, DependencyValue, KeyKind, NodeKind, ValueKind};
    use super::*;
    use ls_types::Range;

    fn make_range(start_line: u32, start_char: u32, end_line: u32, end_char: u32) -> Range {
        Range {
            start: Position {
                line: start_line,
                character: start_char,
            },
            end: Position {
                line: end_line,
                character: end_char,
            },
        }
    }

    fn make_node(id: &str, range: Range, kind: NodeKind) -> TomlNode {
        TomlNode::new(id.to_string(), range, id.to_string(), kind)
    }

    #[test]
    fn test_insert_and_get() {
        let mut tree = SymbolTree::new();
        let node = make_node(
            "test",
            make_range(0, 0, 0, 10),
            NodeKind::Key(KeyKind::Dependency(DependencyKey::CrateName)),
        );
        tree.insert(node);
        tree.build_index();

        assert!(tree.get("test").is_some());
        assert!(tree.get("nonexistent").is_none());
    }

    #[test]
    fn test_find_at_position_simple() {
        let mut tree = SymbolTree::new();

        // Node spanning line 0, chars 0-10
        tree.insert(make_node(
            "a",
            make_range(0, 0, 0, 10),
            NodeKind::Key(KeyKind::Dependency(DependencyKey::CrateName)),
        ));

        // Node spanning line 1, chars 0-5
        tree.insert(make_node(
            "b",
            make_range(1, 0, 1, 5),
            NodeKind::Value(ValueKind::Dependency(DependencyValue::Version)),
        ));

        tree.build_index();

        // Position in first node
        let result = tree.find_at_position(Position {
            line: 0,
            character: 5,
        });
        assert!(result.is_some());
        assert_eq!(result.unwrap().id, "a");

        // Position in second node
        let result = tree.find_at_position(Position {
            line: 1,
            character: 2,
        });
        assert!(result.is_some());
        assert_eq!(result.unwrap().id, "b");

        // Position outside both nodes
        let result = tree.find_at_position(Position {
            line: 2,
            character: 0,
        });
        assert!(result.is_none());
    }

    #[test]
    fn test_find_at_position_overlapping_prefers_key() {
        let mut tree = SymbolTree::new();

        // Larger entry node
        tree.insert(make_node(
            "entry",
            make_range(0, 0, 0, 20),
            NodeKind::Value(ValueKind::Dependency(DependencyValue::Simple)),
        ));

        // Smaller key node at same position
        tree.insert(make_node(
            "key",
            make_range(0, 0, 0, 5),
            NodeKind::Key(KeyKind::Dependency(DependencyKey::CrateName)),
        ));

        tree.build_index();

        // Should prefer the key node
        let result = tree.find_at_position(Position {
            line: 0,
            character: 2,
        });
        assert!(result.is_some());
        assert_eq!(result.unwrap().id, "key");
    }

    #[test]
    fn test_find_at_position_wide_node_spanning_narrow_ones() {
        // A wide multi-line node whose range covers many later narrow nodes.
        // The backwards scan must not stop at the narrow nodes' small ends;
        // the prefix max-end carries the wide node's end forward.
        let mut tree = SymbolTree::new();

        tree.insert(make_node(
            "table",
            make_range(0, 0, 10, 0),
            NodeKind::Value(ValueKind::Dependency(DependencyValue::Table)),
        ));
        for i in 1..=5 {
            tree.insert(make_node(
                &format!("dep{}", i),
                make_range(i, 0, i, 5),
                NodeKind::Value(ValueKind::Dependency(DependencyValue::Version)),
            ));
        }

        tree.build_index();

        // Position on line 8 is inside only the wide table node
        let result = tree.find_at_position(Position {
            line: 8,
            character: 0,
        });
        assert_eq!(result.unwrap().id, "table");

        // Position inside both prefers the narrower node
        let result = tree.find_at_position(Position {
            line: 3,
            character: 2,
        });
        assert_eq!(result.unwrap().id, "dep3");

        // Position past every node matches nothing
        let result = tree.find_at_position(Position {
            line: 10,
            character: 1,
        });
        assert!(result.is_none());
    }

    #[test]
    fn test_find_at_position_narrowest() {
        let mut tree = SymbolTree::new();

        // Larger node
        tree.insert(make_node(
            "outer",
            make_range(0, 0, 0, 30),
            NodeKind::Value(ValueKind::Dependency(DependencyValue::Table)),
        ));

        // Smaller nested node (same type, narrower)
        tree.insert(make_node(
            "inner",
            make_range(0, 10, 0, 20),
            NodeKind::Value(ValueKind::Dependency(DependencyValue::Version)),
        ));

        tree.build_index();

        // Position in inner node should return inner
        let result = tree.find_at_position(Position {
            line: 0,
            character: 15,
        });
        assert!(result.is_some());
        assert_eq!(result.unwrap().id, "inner");

        // Position in outer but not inner should return outer
        let result = tree.find_at_position(Position {
            line: 0,
            character: 5,
        });
        assert!(result.is_some());
        assert_eq!(result.unwrap().id, "outer");
    }
}
