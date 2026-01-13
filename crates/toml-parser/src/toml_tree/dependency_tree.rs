use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::node::DependencyKey;

/// Which dependency table this belongs to
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum DependencyTable {
    /// [dependencies]
    #[default]
    Dependencies,
    /// [dev-dependencies]
    DevDependencies,
    /// [build-dependencies]
    BuildDependencies,
}

impl DependencyTable {
    /// Parse from table name string
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "dependencies" => Some(Self::Dependencies),
            "dev-dependencies" => Some(Self::DevDependencies),
            "build-dependencies" => Some(Self::BuildDependencies),
            _ => None,
        }
    }

    /// Get the table name as a string
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Dependencies => "dependencies",
            Self::DevDependencies => "dev-dependencies",
            Self::BuildDependencies => "build-dependencies",
        }
    }
}

/// Simple "serde = 1.0" vs Table "serde = { version = ... }"
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DependencyStyle {
    /// Simple version string: `serde = "1.0"`
    Simple,
    /// Table with fields: `serde = { version = "1.0", features = [...] }`
    Table,
}

/// Value for a dependency field, with reference to SymbolTree node for position
#[derive(Debug, Clone)]
pub struct FieldValue {
    /// Reference back to SymbolTree for range/position lookup
    pub node_id: String,
    /// The actual value as text
    pub text: String,
}

impl FieldValue {
    pub fn new(node_id: String, text: String) -> Self {
        Self { node_id, text }
    }
}

/// A single feature value with its node reference
#[derive(Debug, Clone)]
pub struct FeatureEntry {
    /// Reference to the feature node in SymbolTree
    pub node_id: String,
    /// The feature name
    pub name: String,
}

impl FeatureEntry {
    pub fn new(node_id: String, name: String) -> Self {
        Self { node_id, name }
    }
}

/// A parsed dependency with O(1) field lookup
#[derive(Debug, Clone)]
pub struct Dependency {
    /// Unique ID for this dependency, e.g., "dependencies.serde"
    pub id: String,
    /// The crate name as declared (may differ from package name)
    pub name: String,
    /// Which table this belongs to
    pub table: DependencyTable,
    /// Simple or table style
    pub style: DependencyStyle,
    /// Platform filter for target-specific deps, e.g., "x86_64-pc-windows-gnu"
    pub platform: Option<String>,
    /// Node ID for the crate name key
    pub name_node_id: String,
    /// Node ID for the entire dependency entry
    pub entry_node_id: String,
    /// O(1) field lookup (uses DependencyKey, excluding CrateName which is tracked via name_node_id)
    pub fields: HashMap<DependencyKey, FieldValue>,
    /// Features list (separate because it's an array of values)
    pub features: Vec<FeatureEntry>,
}

impl Dependency {
    /// Create a new dependency
    pub fn new(
        id: String,
        name: String,
        table: DependencyTable,
        style: DependencyStyle,
        name_node_id: String,
        entry_node_id: String,
    ) -> Self {
        Self {
            id,
            name,
            table,
            style,
            platform: None,
            name_node_id,
            entry_node_id,
            fields: HashMap::new(),
            features: Vec::new(),
        }
    }

    /// Get the version field value
    pub fn version(&self) -> Option<&FieldValue> {
        self.fields.get(&DependencyKey::Version)
    }

    /// Get the git field value
    pub fn git(&self) -> Option<&FieldValue> {
        self.fields.get(&DependencyKey::Git)
    }

    /// Get the path field value
    pub fn path(&self) -> Option<&FieldValue> {
        self.fields.get(&DependencyKey::Path)
    }

    /// Get the workspace field value
    pub fn workspace(&self) -> Option<&FieldValue> {
        self.fields.get(&DependencyKey::Workspace)
    }

    /// Check if this is a workspace dependency
    pub fn is_workspace(&self) -> bool {
        self.fields
            .get(&DependencyKey::Workspace)
            .map(|v| v.text == "true")
            .unwrap_or(false)
    }

    /// Check if this is a git dependency
    pub fn is_git(&self) -> bool {
        self.fields.contains_key(&DependencyKey::Git)
    }

    /// Check if this is a path dependency
    pub fn is_path(&self) -> bool {
        self.fields.contains_key(&DependencyKey::Path)
    }

    /// Get the actual package name (considering package rename)
    pub fn package_name(&self) -> &str {
        self.fields
            .get(&DependencyKey::Package)
            .map(|v| v.text.as_str())
            .unwrap_or(&self.name)
    }

    /// Insert a field value
    pub fn insert_field(&mut self, field: DependencyKey, value: FieldValue) {
        self.fields.insert(field, value);
    }

    /// Add a feature entry
    pub fn add_feature(&mut self, entry: FeatureEntry) {
        self.features.push(entry);
    }
}

/// All dependencies keyed by dotted ID
#[derive(Debug, Clone, Default)]
pub struct DependencyTree {
    /// Primary index: lookup by dependency ID (e.g., "dependencies.serde")
    deps: HashMap<String, Dependency>,
    /// Secondary index: lookup by package name (for audit matching)
    /// Maps package_name -> Vec<dependency_id>
    by_package_name: HashMap<String, Vec<String>>,
}

impl DependencyTree {
    /// Create a new empty DependencyMap
    pub fn new() -> Self {
        Self {
            deps: HashMap::new(),
            by_package_name: HashMap::new(),
        }
    }

    /// Create with pre-allocated capacity
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            deps: HashMap::with_capacity(capacity),
            by_package_name: HashMap::with_capacity(capacity),
        }
    }

    /// Insert a dependency
    pub fn insert(&mut self, dep: Dependency) {
        let package_name = dep.package_name().to_string();
        let dep_id = dep.id.clone();

        // Update secondary index
        self.by_package_name
            .entry(package_name)
            .or_default()
            .push(dep_id.clone());

        self.deps.insert(dep_id, dep);
    }

    /// Get a dependency by ID
    pub fn get(&self, id: &str) -> Option<&Dependency> {
        self.deps.get(id)
    }

    /// Get a mutable reference to a dependency by ID
    pub fn get_mut(&mut self, id: &str) -> Option<&mut Dependency> {
        self.deps.get_mut(id)
    }

    /// Check if a dependency exists
    pub fn contains(&self, id: &str) -> bool {
        self.deps.contains_key(id)
    }

    /// Get the number of dependencies
    pub fn len(&self) -> usize {
        self.deps.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.deps.is_empty()
    }

    /// Iterate over all dependencies
    pub fn iter(&self) -> impl Iterator<Item = (&String, &Dependency)> {
        self.deps.iter()
    }

    /// Iterate over all dependency IDs
    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.deps.keys()
    }

    /// Iterate over all dependencies (values only)
    pub fn values(&self) -> impl Iterator<Item = &Dependency> {
        self.deps.values()
    }

    /// Find dependencies by crate name (declared name in Cargo.toml)
    pub fn find_by_name(&self, name: &str) -> Vec<&Dependency> {
        self.deps.values().filter(|d| d.name == name).collect()
    }

    /// Find dependencies by package name (considering renames).
    /// O(1) lookup using secondary index.
    pub fn find_by_package_name(&self, name: &str) -> Vec<&Dependency> {
        self.by_package_name
            .get(name)
            .map(|ids| ids.iter().filter_map(|id| self.deps.get(id)).collect())
            .unwrap_or_default()
    }

    /// Check if any dependency with the given package name exists.
    /// O(1) lookup.
    pub fn has_package(&self, name: &str) -> bool {
        self.by_package_name.contains_key(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dependency_table_from_str() {
        assert_eq!(
            DependencyTable::from_str("dependencies"),
            Some(DependencyTable::Dependencies)
        );
        assert_eq!(
            DependencyTable::from_str("dev-dependencies"),
            Some(DependencyTable::DevDependencies)
        );
        assert_eq!(
            DependencyTable::from_str("build-dependencies"),
            Some(DependencyTable::BuildDependencies)
        );
        assert_eq!(DependencyTable::from_str("unknown"), None);
    }

    #[test]
    fn test_dependency_creation() {
        let mut dep = Dependency::new(
            "dependencies.serde".to_string(),
            "serde".to_string(),
            DependencyTable::Dependencies,
            DependencyStyle::Table,
            "dependencies.serde.key".to_string(),
            "dependencies.serde".to_string(),
        );

        dep.insert_field(
            DependencyKey::Version,
            FieldValue::new("dependencies.serde.version".to_string(), "1.0".to_string()),
        );

        assert_eq!(dep.name, "serde");
        assert_eq!(dep.version().map(|v| v.text.as_str()), Some("1.0"));
        assert_eq!(dep.package_name(), "serde");
    }

    #[test]
    fn test_dependency_with_package_rename() {
        let mut dep = Dependency::new(
            "dependencies.serde_json".to_string(),
            "serde_json".to_string(),
            DependencyTable::Dependencies,
            DependencyStyle::Table,
            "dependencies.serde_json.key".to_string(),
            "dependencies.serde_json".to_string(),
        );

        dep.insert_field(
            DependencyKey::Package,
            FieldValue::new(
                "dependencies.serde_json.package".to_string(),
                "serde-json".to_string(),
            ),
        );

        assert_eq!(dep.name, "serde_json");
        assert_eq!(dep.package_name(), "serde-json");
    }

    #[test]
    fn test_dependency_map() {
        let mut map = DependencyTree::new();

        let dep = Dependency::new(
            "dependencies.serde".to_string(),
            "serde".to_string(),
            DependencyTable::Dependencies,
            DependencyStyle::Simple,
            "dependencies.serde.key".to_string(),
            "dependencies.serde".to_string(),
        );

        map.insert(dep);

        assert!(map.contains("dependencies.serde"));
        assert!(!map.contains("dependencies.tokio"));
        assert_eq!(map.len(), 1);

        let found = map.find_by_name("serde");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "serde");
    }
}
