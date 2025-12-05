use tower_lsp::lsp_types::Range;

/// A node in the TOML tree with its position
#[derive(Debug, Clone)]
pub struct TomlNode {
    /// Unique dotted key identifier, e.g., "dependencies.serde.version"
    pub id: String,
    /// LSP position range in the document
    pub range: Range,
    /// Raw text content of this node
    pub text: String,
    /// The semantic type of this node
    pub kind: NodeKind,
}

impl TomlNode {
    pub fn new(id: String, range: Range, text: String, kind: NodeKind) -> Self {
        Self {
            id,
            range,
            text,
            kind,
        }
    }

    /// Returns true if this node is a key (left side of =)
    pub fn is_key(&self) -> bool {
        matches!(self.kind, NodeKind::Key(_))
    }

    /// Returns true if this node is a value/entry (right side of =)
    pub fn is_value(&self) -> bool {
        matches!(self.kind, NodeKind::Value(_))
    }

    /// Get the key kind if this is a key node
    pub fn key_kind(&self) -> Option<&KeyKind> {
        match &self.kind {
            NodeKind::Key(k) => Some(k),
            _ => None,
        }
    }

    /// Get the value kind if this is a value node
    pub fn value_kind(&self) -> Option<&ValueKind> {
        match &self.kind {
            NodeKind::Value(v) => Some(v),
            _ => None,
        }
    }

    /// Calculate the width of this node (characters)
    pub fn width(&self) -> u32 {
        if self.range.start.line == self.range.end.line {
            self.range.end.character.saturating_sub(self.range.start.character)
        } else {
            // Multi-line nodes: return a large width
            u32::MAX
        }
    }

    /// Returns the crate name if this is a crate name key node.
    /// Used for completion on crate names.
    pub fn crate_name(&self) -> Option<&str> {
        match &self.kind {
            NodeKind::Key(KeyKind::Dependency(DependencyKey::CrateName)) => Some(&self.text),
            _ => None,
        }
    }
}

// ============================================================================
// Top-level NodeKind
// ============================================================================

/// Node type - either a key or a value
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeKind {
    Key(KeyKind),
    Value(ValueKind),
}

impl NodeKind {
    /// Returns true if this is a key node
    pub fn is_key(&self) -> bool {
        matches!(self, NodeKind::Key(_))
    }

    /// Returns true if this is a value node
    pub fn is_value(&self) -> bool {
        matches!(self, NodeKind::Value(_))
    }

    /// Returns true if this is a top-level dependency entry
    pub fn is_dependency(&self) -> bool {
        matches!(
            self,
            NodeKind::Value(ValueKind::Dependency(
                DependencyValue::Simple | DependencyValue::Table
            ))
        )
    }
}

// ============================================================================
// KeyKind - organized by Cargo.toml section
// ============================================================================

/// Key kinds - nodes that appear on the left side of `=`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyKind {
    /// Keys in dependency tables ([dependencies], [dev-dependencies], etc.)
    Dependency(DependencyKey),
    /// Keys in [package] table
    Package(PackageKey),
    /// Keys in [workspace] table
    Workspace(WorkspaceKey),
    /// Keys in [profile.*] tables
    Profile(ProfileKey),
    /// Keys in [features] table
    Features(FeaturesKey),
    /// Generic/unknown key
    Other,
}

/// Keys specific to dependency declarations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DependencyKey {
    /// The crate name key, e.g., `serde` in `serde = "1.0"`
    CrateName,
    /// The `version` key
    Version,
    /// The `features` key
    Features,
    /// The `git` key
    Git,
    /// The `path` key
    Path,
    /// The `branch` key
    Branch,
    /// The `tag` key
    Tag,
    /// The `rev` key
    Rev,
    /// The `workspace` key
    Workspace,
    /// The `registry` key
    Registry,
    /// The `package` key (for renaming)
    Package,
    /// The `default-features` key
    DefaultFeatures,
    /// The `optional` key
    Optional,
}

/// Keys specific to [package] table
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PackageKey {
    Name,
    Version,
    Authors,
    Edition,
    Description,
    Documentation,
    Readme,
    Homepage,
    Repository,
    License,
    LicenseFile,
    Keywords,
    Categories,
    Workspace,
    Build,
    Links,
    Exclude,
    Include,
    Publish,
    DefaultRun,
    Autobins,
    Autoexamples,
    Autotests,
    Autobenches,
    Resolver,
    Other,
}

/// Keys specific to [workspace] table
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkspaceKey {
    Members,
    Exclude,
    Resolver,
    Dependencies,
    Package,
    Other,
}

/// Keys specific to [profile.*] tables
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProfileKey {
    OptLevel,
    Debug,
    DebugAssertions,
    OverflowChecks,
    Lto,
    Panic,
    IncrementalCompilation,
    CodegenUnits,
    Rpath,
    Other,
}

/// Keys specific to [features] table
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FeaturesKey {
    /// A feature name
    FeatureName,
    /// The `default` feature
    Default,
}

// ============================================================================
// ValueKind - organized by Cargo.toml section
// ============================================================================

/// Value kinds - nodes that appear on the right side of `=` or as table content
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValueKind {
    /// Values in dependency tables
    Dependency(DependencyValue),
    /// Values in [package] table
    Package(PackageValue),
    /// Values in [workspace] table
    Workspace(WorkspaceValue),
    /// Values in [profile.*] tables
    Profile(ProfileValue),
    /// Values in [features] table
    Features(FeaturesValue),
    /// Generic table
    Table,
    /// Generic array
    Array,
    /// Generic value (string, number, bool, etc.)
    Other,
}

/// Values specific to dependency declarations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DependencyValue {
    /// Simple dependency: `serde = "1.0"`
    Simple,
    /// Table dependency: `serde = { version = "1.0" }`
    Table,
    /// Version value string
    Version,
    /// Features array: `["derive", "std"]`
    FeaturesArray,
    /// A single feature value within the features array
    Feature,
    /// Git URL value
    Git,
    /// Path value
    Path,
    /// Branch value
    Branch,
    /// Tag value
    Tag,
    /// Rev (revision) value
    Rev,
    /// Workspace boolean value
    Workspace,
    /// Registry value
    Registry,
    /// Package (rename) value
    Package,
    /// Default-features boolean value
    DefaultFeatures,
    /// Optional boolean value
    Optional,
}

/// Values specific to [package] table
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PackageValue {
    Name,
    Version,
    Authors,
    Edition,
    Description,
    Other,
}

/// Values specific to [workspace] table
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkspaceValue {
    Members,
    Exclude,
    Other,
}

/// Values specific to [profile.*] tables
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProfileValue {
    OptLevel,
    Debug,
    Lto,
    Other,
}

/// Values specific to [features] table
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FeaturesValue {
    /// Feature dependencies array
    FeatureDeps,
    /// A single feature dependency
    FeatureDep,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_kind_is_key() {
        assert!(NodeKind::Key(KeyKind::Dependency(DependencyKey::CrateName)).is_key());
        assert!(NodeKind::Key(KeyKind::Dependency(DependencyKey::Version)).is_key());
        assert!(!NodeKind::Value(ValueKind::Dependency(DependencyValue::Simple)).is_key());
        assert!(!NodeKind::Value(ValueKind::Dependency(DependencyValue::Version)).is_key());
    }

    #[test]
    fn test_node_kind_is_dependency() {
        assert!(NodeKind::Value(ValueKind::Dependency(DependencyValue::Simple)).is_dependency());
        assert!(NodeKind::Value(ValueKind::Dependency(DependencyValue::Table)).is_dependency());
        assert!(!NodeKind::Value(ValueKind::Dependency(DependencyValue::Version)).is_dependency());
        assert!(!NodeKind::Key(KeyKind::Dependency(DependencyKey::CrateName)).is_dependency());
    }

    #[test]
    fn test_key_kind_extraction() {
        let node = TomlNode::new(
            "test".to_string(),
            Range::default(),
            "test".to_string(),
            NodeKind::Key(KeyKind::Dependency(DependencyKey::Version)),
        );
        assert_eq!(
            node.key_kind(),
            Some(&KeyKind::Dependency(DependencyKey::Version))
        );
        assert_eq!(node.value_kind(), None);
    }

    #[test]
    fn test_value_kind_extraction() {
        let node = TomlNode::new(
            "test".to_string(),
            Range::default(),
            "1.0".to_string(),
            NodeKind::Value(ValueKind::Dependency(DependencyValue::Version)),
        );
        assert_eq!(node.key_kind(), None);
        assert_eq!(
            node.value_kind(),
            Some(&ValueKind::Dependency(DependencyValue::Version))
        );
    }
}
