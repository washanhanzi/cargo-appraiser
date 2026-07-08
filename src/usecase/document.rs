use std::collections::HashMap;

use tower_lsp::lsp_types::{Position, Uri};

use crate::entity::{
    CanonicalUri, ResolvedDependency, TomlDependency, TomlNode, TomlTree, WorkspaceMember,
};

/// Represents a parsed Cargo.toml document with resolution status.
#[derive(Debug, Clone)]
pub struct Document {
    pub uri: Uri,
    pub canonical_uri: CanonicalUri,
    pub rev: usize,
    /// The parsed TOML tree (from toml-parser)
    tree: TomlTree,
    /// Cargo resolution results, keyed by dependency ID
    pub resolved: HashMap<String, ResolvedDependency>,
    /// Dependencies that need re-resolution, maps dep_id -> rev when marked dirty
    pub dirty_dependencies: HashMap<String, usize>,
    /// Parsing errors from toml-parser
    pub parsing_errors: Vec<toml_parser::ParseError>,
    /// Workspace members (populated after cargo resolution)
    pub members: Option<Vec<WorkspaceMember>>,
}

impl Document {
    /// Get the underlying TomlTree
    pub fn tree(&self) -> &TomlTree {
        &self.tree
    }

    /// Parse a Cargo.toml document
    pub fn parse(uri: Uri, canonical_uri: CanonicalUri, text: &str) -> Self {
        let result = toml_parser::parse(text);

        Self {
            uri,
            canonical_uri,
            rev: 0,
            tree: result.tree,
            resolved: HashMap::new(),
            dirty_dependencies: HashMap::new(),
            parsing_errors: result.errors,
            members: None,
        }
    }

    /// Check if any dependencies need resolution
    pub fn is_dependencies_dirty(&self) -> bool {
        !self.dirty_dependencies.is_empty()
    }

    /// Find the most specific node at the given position
    pub fn precise_match(&self, pos: Position) -> Option<&TomlNode> {
        self.tree.find_at_position(pos)
    }

    /// Get a dependency by its ID (e.g., "dependencies.serde")
    pub fn dependency(&self, id: &str) -> Option<&TomlDependency> {
        if id.is_empty() {
            return None;
        }
        self.tree.get_dependency(id)
    }

    /// Get all dependencies with the given crate name
    pub fn dependencies_by_crate_name(&self, crate_name: &str) -> Vec<&TomlDependency> {
        self.tree.find_dependencies_by_name(crate_name)
    }

    /// Get the entry node for a dependency
    pub fn entry(&self, dep_id: &str) -> Option<&TomlNode> {
        let dep = self.tree.get_dependency(dep_id)?;
        self.tree.get_node(&dep.entry_node_id)
    }

    /// Get the name node (crate name key) for a dependency
    pub fn name_node(&self, dep_id: &str) -> Option<&TomlNode> {
        let dep = self.tree.get_dependency(dep_id)?;
        self.tree.get_node(&dep.name_node_id)
    }

    /// Find all key nodes with the given crate name text
    pub fn find_keys_by_crate_name(&self, crate_name: &str) -> Vec<&TomlNode> {
        self.tree
            .nodes()
            .filter(|n| n.is_key() && n.text == crate_name)
            .collect()
    }

    /// Find all dependencies matching a crate name
    pub fn find_deps_by_crate_name(&self, crate_name: &str) -> Vec<&TomlDependency> {
        self.tree
            .dependencies()
            .filter(|d| d.package_name() == crate_name)
            .collect()
    }

    /// Get the resolved dependency info for a dependency ID
    pub fn resolved(&self, dep_id: &str) -> Option<&ResolvedDependency> {
        self.resolved.get(dep_id)
    }

    /// Mark all dependencies as dirty (needing resolution)
    pub fn mark_dirty(&mut self) {
        self.rev += 1;
        for dep in self.tree.dependencies() {
            self.dirty_dependencies.insert(dep.id.clone(), self.rev);
        }
    }

    /// Mark a specific dependency as resolved (no longer dirty)
    pub fn mark_resolved(&mut self, dep_id: &str) {
        self.dirty_dependencies.remove(dep_id);
    }

    /// Store resolution result for a dependency
    pub fn set_resolved(&mut self, dep_id: &str, resolved: ResolvedDependency) {
        self.resolved.insert(dep_id.to_string(), resolved);
    }

    /// Get all dependency IDs
    pub fn dependency_ids(&self) -> impl Iterator<Item = &String> {
        self.tree.dependencies().map(|d| &d.id)
    }

    /// Get all dependencies
    pub fn dependencies(&self) -> impl Iterator<Item = &TomlDependency> {
        self.tree.dependencies()
    }

    /// Check if a dependency is a workspace dependency (declared in workspace.dependencies)
    pub fn is_workspace_dep(&self, dep: &TomlDependency) -> bool {
        dep.id.starts_with("workspace.dependencies.")
    }
}

#[cfg(test)]
mod tests {
    use tower_lsp::lsp_types::Uri;

    use super::Document;
    use crate::entity::{DependencyStyle, DependencyTable};

    #[test]
    fn test_parse() {
        // Use a platform-agnostic approach with a temp file
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join("test_cargo_appraiser.toml");

        // Create the temp file so canonicalization works
        std::fs::write(&temp_file, "").unwrap();

        let uri = Uri::try_from_path(&temp_file).unwrap();
        let canonical_uri = uri.clone().try_into().unwrap();

        // Clean up the temp file
        std::fs::remove_file(&temp_file).unwrap();

        let doc = Document::parse(
            uri,
            canonical_uri,
            r#"
            [dependencies]
            a = "0.1.0"
            "#,
        );

        assert_eq!(doc.dependencies().count(), 1);

        let dep = doc.dependency("dependencies.a").unwrap();
        assert_eq!(dep.name, "a");
        assert_eq!(dep.table, DependencyTable::Dependencies);
        assert_eq!(dep.style, DependencyStyle::Simple);
        assert_eq!(dep.version().map(|v| v.text.as_str()), Some("0.1.0"));
    }

    #[test]
    fn test_find_deps_by_crate_name() {
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join("test_cargo_appraiser2.toml");
        std::fs::write(&temp_file, "").unwrap();

        let uri = Uri::try_from_path(&temp_file).unwrap();
        let canonical_uri = uri.clone().try_into().unwrap();
        std::fs::remove_file(&temp_file).unwrap();

        let doc = Document::parse(
            uri,
            canonical_uri,
            r#"
            [dependencies]
            serde = "1.0"

            [dev-dependencies]
            serde = "1.0"
            "#,
        );

        let deps = doc.find_deps_by_crate_name("serde");
        assert_eq!(deps.len(), 2);
    }

    #[test]
    fn test_name_node() {
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join("test_cargo_appraiser_name_node.toml");
        std::fs::write(&temp_file, "").unwrap();

        let uri = Uri::try_from_path(&temp_file).unwrap();
        let canonical_uri = uri.clone().try_into().unwrap();
        std::fs::remove_file(&temp_file).unwrap();

        let doc = Document::parse(
            uri,
            canonical_uri,
            r#"
            [dependencies]
            serde = "1.0"
            tokio = { version = "1.0", features = ["full"] }
            "#,
        );

        // Test name_node for simple dependency
        let name_node = doc.name_node("dependencies.serde");
        assert!(name_node.is_some());
        let node = name_node.unwrap();
        assert_eq!(node.text, "serde");

        // Test name_node for table dependency
        let name_node = doc.name_node("dependencies.tokio");
        assert!(name_node.is_some());
        let node = name_node.unwrap();
        assert_eq!(node.text, "tokio");

        // Test name_node returns None for non-existent dependency
        let name_node = doc.name_node("dependencies.nonexistent");
        assert!(name_node.is_none());
    }

    #[test]
    fn test_entry_node() {
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join("test_cargo_appraiser_entry.toml");
        std::fs::write(&temp_file, "").unwrap();

        let uri = Uri::try_from_path(&temp_file).unwrap();
        let canonical_uri = uri.clone().try_into().unwrap();
        std::fs::remove_file(&temp_file).unwrap();

        let doc = Document::parse(
            uri,
            canonical_uri,
            r#"
            [dependencies]
            serde = "1.0"
            "#,
        );

        // Entry node should exist for valid dependency
        let entry = doc.entry("dependencies.serde");
        assert!(entry.is_some());

        // Entry should not exist for invalid dependency
        let entry = doc.entry("dependencies.invalid");
        assert!(entry.is_none());

        // Empty dep_id should return None
        let dep = doc.dependency("");
        assert!(dep.is_none());
    }
}
