mod dependency_tree;
mod node;
mod symbol_tree;

use tower_lsp::lsp_types::Position;

// Re-export public types
pub use dependency_tree::{Dependency, DependencyStyle, DependencyTable, DependencyTree, FieldValue};
pub use node::{
    DependencyKey, DependencyValue, KeyKind, NodeKind, TomlNode, ValueKind, WorkspaceKey,
    WorkspaceValue,
};
pub use symbol_tree::SymbolTree;

// Internal re-exports for sibling modules
pub(crate) use dependency_tree::FeatureEntry;

/// The complete parsed Cargo.toml structure.
///
/// Combines:
/// - `SymbolTree`: All TOML nodes with position lookup
/// - `DependencyTree`: Semantic dependency information
#[derive(Debug, Clone, Default)]
pub struct TomlTree {
    /// All TOML nodes for position-based lookups
    symbols: SymbolTree,
    /// Parsed dependencies for semantic lookups
    dependencies: DependencyTree,
}

impl TomlTree {
    /// Create a new empty TomlTree
    pub fn new() -> Self {
        Self {
            symbols: SymbolTree::new(),
            dependencies: DependencyTree::new(),
        }
    }

    /// Create with pre-allocated capacity
    pub fn with_capacity(symbols_capacity: usize, deps_capacity: usize) -> Self {
        Self {
            symbols: SymbolTree::with_capacity(symbols_capacity),
            dependencies: DependencyTree::with_capacity(deps_capacity),
        }
    }

    /// Build the position index. Must be called after all insertions.
    pub fn build_index(&mut self) {
        self.symbols.build_index();
    }

    // ========================================================================
    // Symbol tree operations
    // ========================================================================

    /// Insert a TOML node
    pub fn insert_node(&mut self, node: TomlNode) {
        self.symbols.insert(node);
    }

    /// Get a node by its dotted ID
    pub fn get_node(&self, id: &str) -> Option<&TomlNode> {
        self.symbols.get(id)
    }

    /// Check if a node exists
    pub fn contains_node(&self, id: &str) -> bool {
        self.symbols.contains(id)
    }

    /// Find the most specific node at the given position
    pub fn find_at_position(&self, pos: Position) -> Option<&TomlNode> {
        self.symbols.find_at_position(pos)
    }

    /// Find a key node at the given position
    pub fn find_key_at_position(&self, pos: Position) -> Option<&TomlNode> {
        self.symbols.find_key_at_position(pos)
    }

    /// Find a value node at the given position
    pub fn find_value_at_position(&self, pos: Position) -> Option<&TomlNode> {
        self.symbols.find_value_at_position(pos)
    }

    /// Iterate over all nodes
    pub fn nodes(&self) -> impl Iterator<Item = &TomlNode> {
        self.symbols.values()
    }

    // ========================================================================
    // Dependency tree operations
    // ========================================================================

    /// Insert a dependency
    pub fn insert_dependency(&mut self, dep: Dependency) {
        self.dependencies.insert(dep);
    }

    /// Get a dependency by its dotted ID
    pub fn get_dependency(&self, id: &str) -> Option<&Dependency> {
        self.dependencies.get(id)
    }

    /// Get a mutable reference to a dependency
    pub fn get_dependency_mut(&mut self, id: &str) -> Option<&mut Dependency> {
        self.dependencies.get_mut(id)
    }

    /// Check if a dependency exists
    pub fn contains_dependency(&self, id: &str) -> bool {
        self.dependencies.contains(id)
    }

    /// Get the number of dependencies
    pub fn dependency_count(&self) -> usize {
        self.dependencies.len()
    }

    /// Iterate over all dependencies
    pub fn dependencies(&self) -> impl Iterator<Item = &Dependency> {
        self.dependencies.values()
    }

    /// Find dependencies by crate name
    pub fn find_dependencies_by_name(&self, name: &str) -> Vec<&Dependency> {
        self.dependencies.find_by_name(name)
    }

    /// Find dependencies by package name (considering renames)
    pub fn find_dependencies_by_package_name(&self, name: &str) -> Vec<&Dependency> {
        self.dependencies.find_by_package_name(name)
    }

    // ========================================================================
    // Combined operations
    // ========================================================================

    /// Find the dependency at the given position (if any)
    pub fn find_dependency_at_position(&self, pos: Position) -> Option<&Dependency> {
        let node = self.find_at_position(pos)?;

        // Check if this node is part of a dependency
        // The node ID format is like "dependencies.serde" or "dependencies.serde.version"
        let parts: Vec<&str> = node.id.split('.').collect();
        if parts.len() >= 2 {
            // Try to find the dependency by progressively shorter prefixes
            // e.g., for "dependencies.serde.version", try:
            //   - "dependencies.serde.version" (not a dep)
            //   - "dependencies.serde" (this is a dep)
            for i in (2..=parts.len()).rev() {
                let dep_id = parts[..i].join(".");
                if let Some(dep) = self.dependencies.get(&dep_id) {
                    return Some(dep);
                }
            }
        }

        None
    }

    /// Get the node for a dependency's name key
    pub fn get_dependency_name_node(&self, dep: &Dependency) -> Option<&TomlNode> {
        self.symbols.get(&dep.name_node_id)
    }

    /// Get the node for a dependency's entry value
    pub fn get_dependency_entry_node(&self, dep: &Dependency) -> Option<&TomlNode> {
        self.symbols.get(&dep.entry_node_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::walker::parse;

    #[test]
    fn test_find_dependency_at_position() {
        let toml = r#"[dependencies]
serde = "1.0"
"#;
        let result = parse(toml);

        // Position at "serde" key
        let dep = result.tree.find_dependency_at_position(Position {
            line: 1,
            character: 2,
        });
        assert!(dep.is_some());
        assert_eq!(dep.unwrap().name, "serde");

        // Position at version value
        let dep = result.tree.find_dependency_at_position(Position {
            line: 1,
            character: 10,
        });
        assert!(dep.is_some());
        assert_eq!(dep.unwrap().name, "serde");
    }

    #[test]
    fn test_combined_lookups() {
        let toml = r#"[dependencies]
serde = { version = "1.0", features = ["derive"] }
"#;
        let result = parse(toml);

        let dep = result.tree.get_dependency("dependencies.serde").unwrap();
        assert_eq!(dep.name, "serde");

        // Get the name node for this dependency
        let name_node = result.tree.get_dependency_name_node(dep);
        assert!(name_node.is_some());
        assert_eq!(name_node.unwrap().text, "serde");
    }
}
