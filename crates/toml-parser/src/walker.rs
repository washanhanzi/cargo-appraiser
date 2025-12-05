use lsp_async_stub::util::Mapper;
use taplo::{
    dom::{node::Key, Node},
    util::join_ranges,
};
use tower_lsp::lsp_types::{Position, Range};

use crate::toml_tree::{
    Dependency, DependencyKey, DependencyStyle, DependencyTable, DependencyValue, FeatureEntry,
    FieldValue, KeyKind, NodeKind, TomlNode, TomlTree, ValueKind,
};

/// Result of parsing a Cargo.toml file
#[derive(Debug)]
pub struct ParseResult {
    /// The combined TOML tree
    pub tree: TomlTree,
    /// Parsing errors (if any)
    pub errors: Vec<ParseError>,
}

/// A parsing error with position information
#[derive(Debug, Clone)]
pub struct ParseError {
    pub message: String,
    pub range: Range,
}

impl ParseError {
    pub fn new(message: String, range: Range) -> Self {
        Self { message, range }
    }
}

/// Parse a Cargo.toml file and return the symbol tree and dependency map
pub fn parse(text: &str) -> ParseResult {
    let parsed = taplo::parser::parse(text);
    let mapper = Mapper::new_utf16(text, false);

    let mut walker = Walker::new(mapper);

    // Get the DOM from parsed TOML
    let dom = parsed.into_dom();

    // Walk the root table
    if let Node::Table(root) = dom {
        let entries = root.entries().read();
        for (key, value) in entries.iter() {
            let table_name = key.value();
            let id = table_name.to_string();
            walker.walk_root(&id, table_name, value, key);
        }
    }

    walker.finish()
}

struct Walker {
    tree: TomlTree,
    mapper: Mapper,
    errors: Vec<ParseError>,
}

impl Walker {
    fn new(mapper: Mapper) -> Self {
        Self {
            tree: TomlTree::with_capacity(64, 32),
            mapper,
            errors: Vec::new(),
        }
    }

    fn finish(mut self) -> ParseResult {
        self.tree.build_index();
        ParseResult {
            tree: self.tree,
            errors: self.errors,
        }
    }

    fn to_range(&self, node: &Node) -> Range {
        let text_range = join_ranges(node.text_ranges(true));
        self.mapper_range_to_lsp(self.mapper.range(text_range).unwrap())
    }

    fn key_to_range(&self, key: &Key) -> Range {
        let text_range = join_ranges(key.text_ranges());
        self.mapper_range_to_lsp(self.mapper.range(text_range).unwrap())
    }

    fn mapper_range_to_lsp(&self, range: lsp_async_stub::util::Range) -> Range {
        Range {
            start: Position {
                line: range.start.line as u32,
                character: range.start.character as u32,
            },
            end: Position {
                line: range.end.line as u32,
                character: range.end.character as u32,
            },
        }
    }

    fn walk_root(&mut self, id: &str, table_name: &str, node: &Node, _key: &Key) {
        // Check if this is a dependency-related table
        if let Some(dep_table) = DependencyTable::from_str(table_name) {
            self.walk_dependency_table(id, dep_table, node, None);
            return;
        }

        // Handle target-specific dependencies: [target.'cfg(...)'.dependencies]
        if table_name == "target" {
            self.walk_target_table(id, node);
            return;
        }

        // Handle workspace table
        if table_name == "workspace" {
            self.walk_workspace_table(id, node);
            return;
        }

        // Generic table handling for other sections
        self.insert_node(id, node, NodeKind::Value(ValueKind::Table));
    }

    fn walk_target_table(&mut self, id: &str, node: &Node) {
        let Node::Table(t) = node else { return };

        let entries = t.entries().read();
        for (platform_key, platform_value) in entries.iter() {
            let platform = platform_key.value();
            let platform_id = format!("{}.{}", id, platform);

            let Node::Table(platform_table) = platform_value else {
                continue;
            };

            let platform_entries = platform_table.entries().read();
            for (dep_table_key, dep_table_value) in platform_entries.iter() {
                let dep_table_name = dep_table_key.value();
                if let Some(dep_table) = DependencyTable::from_str(dep_table_name) {
                    let dep_id = format!("{}.{}", platform_id, dep_table_name);
                    self.walk_dependency_table(&dep_id, dep_table, dep_table_value, Some(platform));
                }
            }
        }
    }

    fn walk_workspace_table(&mut self, id: &str, node: &Node) {
        let Node::Table(t) = node else { return };

        let entries = t.entries().read();
        for (key, value) in entries.iter() {
            let key_name = key.value();
            let entry_id = format!("{}.{}", id, key_name);

            match key_name {
                "dependencies" => {
                    // workspace.dependencies - these are virtual/shared deps
                    self.walk_dependency_table(
                        &entry_id,
                        DependencyTable::Dependencies,
                        value,
                        None,
                    );
                }
                "members" | "exclude" => {
                    // Array values - just record as generic
                    self.insert_node(&entry_id, value, NodeKind::Value(ValueKind::Array));
                }
                _ => {
                    self.insert_node(&entry_id, value, NodeKind::Value(ValueKind::Other));
                }
            }
        }
    }

    fn walk_dependency_table(
        &mut self,
        id: &str,
        table: DependencyTable,
        node: &Node,
        platform: Option<&str>,
    ) {
        let Node::Table(t) = node else { return };

        let entries = t.entries().read();
        for (crate_key, crate_value) in entries.iter() {
            let crate_name = crate_key.value();
            let dep_id = format!("{}.{}", id, crate_name);

            self.walk_dependency(&dep_id, crate_name, crate_key, crate_value, table, platform);
        }
    }

    fn walk_dependency(
        &mut self,
        id: &str,
        crate_name: &str,
        crate_key: &Key,
        node: &Node,
        table: DependencyTable,
        platform: Option<&str>,
    ) {
        let crate_key_id = format!("{}.key", id);
        let crate_key_range = self.key_to_range(crate_key);

        // Insert crate name key node
        self.tree.insert_node(TomlNode::new(
            crate_key_id.clone(),
            crate_key_range,
            crate_name.to_string(),
            NodeKind::Key(KeyKind::Dependency(DependencyKey::CrateName)),
        ));

        match node {
            // Simple dependency: serde = "1.0"
            Node::Str(s) => {
                let range = self.to_range(node);
                let version = s.value().to_string();

                // Insert the simple dependency node
                self.tree.insert_node(TomlNode::new(
                    id.to_string(),
                    range,
                    version.clone(),
                    NodeKind::Value(ValueKind::Dependency(DependencyValue::Simple)),
                ));

                // Create dependency with version field
                let mut dep = Dependency::new(
                    id.to_string(),
                    crate_name.to_string(),
                    table,
                    DependencyStyle::Simple,
                    crate_key_id,
                    id.to_string(),
                );
                dep.platform = platform.map(|s| s.to_string());
                dep.insert_field(
                    DependencyKey::Version,
                    FieldValue::new(id.to_string(), version),
                );

                self.tree.insert_dependency(dep);
            }

            // Table dependency: serde = { version = "1.0", features = [...] }
            Node::Table(t) => {
                let range = self.to_range(node);

                // Insert the table dependency node
                self.tree.insert_node(TomlNode::new(
                    id.to_string(),
                    range,
                    String::new(), // No single text value for table
                    NodeKind::Value(ValueKind::Dependency(DependencyValue::Table)),
                ));

                // Create dependency
                let mut dep = Dependency::new(
                    id.to_string(),
                    crate_name.to_string(),
                    table,
                    DependencyStyle::Table,
                    crate_key_id,
                    id.to_string(),
                );
                dep.platform = platform.map(|s| s.to_string());

                // Walk table entries
                let entries = t.entries().read();
                for (field_key, field_value) in entries.iter() {
                    self.walk_dependency_field(id, field_key, field_value, &mut dep);
                }

                self.tree.insert_dependency(dep);
            }

            // Invalid node
            _ => {
                let range = self.to_range(node);
                self.errors.push(ParseError::new(
                    format!("Invalid dependency format for '{}'", crate_name),
                    range,
                ));
            }
        }
    }

    fn walk_dependency_field(
        &mut self,
        dep_id: &str,
        key: &Key,
        value: &Node,
        dep: &mut Dependency,
    ) {
        let field_name = key.value();
        let field_id = format!("{}.{}", dep_id, field_name);
        let key_id = format!("{}.key", field_id);

        // Determine the node kinds based on field name
        let (key_kind, value_kind, dep_field) = match field_name {
            "version" => (
                NodeKind::Key(KeyKind::Dependency(DependencyKey::Version)),
                NodeKind::Value(ValueKind::Dependency(DependencyValue::Version)),
                Some(DependencyKey::Version),
            ),
            "features" => (
                NodeKind::Key(KeyKind::Dependency(DependencyKey::Features)),
                NodeKind::Value(ValueKind::Dependency(DependencyValue::FeaturesArray)),
                None,
            ),
            "git" => (
                NodeKind::Key(KeyKind::Dependency(DependencyKey::Git)),
                NodeKind::Value(ValueKind::Dependency(DependencyValue::Git)),
                Some(DependencyKey::Git),
            ),
            "path" => (
                NodeKind::Key(KeyKind::Dependency(DependencyKey::Path)),
                NodeKind::Value(ValueKind::Dependency(DependencyValue::Path)),
                Some(DependencyKey::Path),
            ),
            "branch" => (
                NodeKind::Key(KeyKind::Dependency(DependencyKey::Branch)),
                NodeKind::Value(ValueKind::Dependency(DependencyValue::Branch)),
                Some(DependencyKey::Branch),
            ),
            "tag" => (
                NodeKind::Key(KeyKind::Dependency(DependencyKey::Tag)),
                NodeKind::Value(ValueKind::Dependency(DependencyValue::Tag)),
                Some(DependencyKey::Tag),
            ),
            "rev" => (
                NodeKind::Key(KeyKind::Dependency(DependencyKey::Rev)),
                NodeKind::Value(ValueKind::Dependency(DependencyValue::Rev)),
                Some(DependencyKey::Rev),
            ),
            "workspace" => (
                NodeKind::Key(KeyKind::Dependency(DependencyKey::Workspace)),
                NodeKind::Value(ValueKind::Dependency(DependencyValue::Workspace)),
                Some(DependencyKey::Workspace),
            ),
            "registry" => (
                NodeKind::Key(KeyKind::Dependency(DependencyKey::Registry)),
                NodeKind::Value(ValueKind::Dependency(DependencyValue::Registry)),
                Some(DependencyKey::Registry),
            ),
            "package" => (
                NodeKind::Key(KeyKind::Dependency(DependencyKey::Package)),
                NodeKind::Value(ValueKind::Dependency(DependencyValue::Package)),
                Some(DependencyKey::Package),
            ),
            "default-features" => (
                NodeKind::Key(KeyKind::Dependency(DependencyKey::DefaultFeatures)),
                NodeKind::Value(ValueKind::Dependency(DependencyValue::DefaultFeatures)),
                Some(DependencyKey::DefaultFeatures),
            ),
            "optional" => (
                NodeKind::Key(KeyKind::Dependency(DependencyKey::Optional)),
                NodeKind::Value(ValueKind::Dependency(DependencyValue::Optional)),
                Some(DependencyKey::Optional),
            ),
            _ => (
                NodeKind::Key(KeyKind::Other),
                NodeKind::Value(ValueKind::Other),
                None,
            ),
        };

        // Insert key node
        let key_range = self.key_to_range(key);
        self.tree.insert_node(TomlNode::new(
            key_id,
            key_range,
            field_name.to_string(),
            key_kind,
        ));

        // Handle features array specially
        if field_name == "features" {
            self.walk_features_array(&field_id, value, dep);
            return;
        }

        // Insert value node
        let value_range = self.to_range(value);
        let value_text = self.node_to_text(value);
        self.tree.insert_node(TomlNode::new(
            field_id.clone(),
            value_range,
            value_text.clone(),
            value_kind,
        ));

        // Add to dependency fields
        if let Some(field) = dep_field {
            dep.insert_field(field, FieldValue::new(field_id, value_text));
        }
    }

    fn walk_features_array(&mut self, id: &str, node: &Node, dep: &mut Dependency) {
        let range = self.to_range(node);

        // Insert the features array node
        self.tree.insert_node(TomlNode::new(
            id.to_string(),
            range,
            String::new(),
            NodeKind::Value(ValueKind::Dependency(DependencyValue::FeaturesArray)),
        ));

        let Node::Array(arr) = node else { return };

        let items = arr.items().read();
        for (i, item) in items.iter().enumerate() {
            let feature_id = format!("{}.{}", id, i);

            if let Node::Str(s) = item {
                let feature_name = s.value().to_string();
                let feature_range = self.to_range(item);

                self.tree.insert_node(TomlNode::new(
                    feature_id.clone(),
                    feature_range,
                    feature_name.clone(),
                    NodeKind::Value(ValueKind::Dependency(DependencyValue::Feature)),
                ));

                dep.add_feature(FeatureEntry::new(feature_id, feature_name));
            }
        }
    }

    fn insert_node(&mut self, id: &str, node: &Node, kind: NodeKind) {
        let range = self.to_range(node);
        let text = self.node_to_text(node);
        self.tree
            .insert_node(TomlNode::new(id.to_string(), range, text, kind));
    }

    fn node_to_text(&self, node: &Node) -> String {
        match node {
            Node::Str(s) => s.value().to_string(),
            Node::Bool(b) => b.value().to_string(),
            Node::Integer(i) => i.value().to_string(),
            Node::Float(f) => f.value().to_string(),
            _ => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_dependency() {
        let toml = r#"
[dependencies]
serde = "1.0"
"#;
        let result = parse(toml);

        assert!(result.errors.is_empty());
        assert!(result.tree.contains_dependency("dependencies.serde"));

        let dep = result.tree.get_dependency("dependencies.serde").unwrap();
        assert_eq!(dep.name, "serde");
        assert_eq!(dep.style, DependencyStyle::Simple);
        assert_eq!(dep.version().map(|v| v.text.as_str()), Some("1.0"));
    }

    #[test]
    fn test_parse_table_dependency() {
        let toml = r#"
[dependencies]
serde = { version = "1.0", features = ["derive"] }
"#;
        let result = parse(toml);

        assert!(result.errors.is_empty());

        let dep = result.tree.get_dependency("dependencies.serde").unwrap();
        assert_eq!(dep.name, "serde");
        assert_eq!(dep.style, DependencyStyle::Table);
        assert_eq!(dep.version().map(|v| v.text.as_str()), Some("1.0"));
        assert_eq!(dep.features.len(), 1);
        assert_eq!(dep.features[0].name, "derive");
    }

    #[test]
    fn test_parse_dev_dependency() {
        let toml = r#"
[dev-dependencies]
tempfile = "3.0"
"#;
        let result = parse(toml);

        let dep = result
            .tree
            .get_dependency("dev-dependencies.tempfile")
            .unwrap();
        assert_eq!(dep.table, DependencyTable::DevDependencies);
    }

    #[test]
    fn test_parse_target_specific() {
        let toml = r#"
[target.'cfg(windows)'.dependencies]
winapi = "0.3"
"#;
        let result = parse(toml);

        let dep = result
            .tree
            .get_dependency("target.cfg(windows).dependencies.winapi")
            .unwrap();
        assert_eq!(dep.name, "winapi");
        assert_eq!(dep.platform, Some("cfg(windows)".to_string()));
    }

    #[test]
    fn test_position_lookup() {
        let toml = r#"[dependencies]
serde = "1.0"
"#;
        let result = parse(toml);

        // Position at "serde" should find the crate name
        let node = result.tree.find_at_position(Position {
            line: 1,
            character: 2,
        });
        assert!(node.is_some());
        // The key node should be found
        let kind = node.unwrap().kind;
        assert!(
            kind == NodeKind::Key(KeyKind::Dependency(DependencyKey::CrateName))
                || kind == NodeKind::Value(ValueKind::Dependency(DependencyValue::Simple))
        );
    }

    #[test]
    fn test_git_dependency() {
        let toml = r#"
[dependencies]
my-crate = { git = "https://github.com/user/repo", branch = "main" }
"#;
        let result = parse(toml);

        let dep = result.tree.get_dependency("dependencies.my-crate").unwrap();
        assert!(dep.is_git());
        assert_eq!(
            dep.git().map(|v| v.text.as_str()),
            Some("https://github.com/user/repo")
        );
        assert_eq!(
            dep.fields
                .get(&DependencyKey::Branch)
                .map(|v| v.text.as_str()),
            Some("main")
        );
    }

    #[test]
    fn test_workspace_dependency() {
        let toml = r#"
[dependencies]
shared = { workspace = true }
"#;
        let result = parse(toml);

        let dep = result.tree.get_dependency("dependencies.shared").unwrap();
        assert!(dep.is_workspace());
    }
}
