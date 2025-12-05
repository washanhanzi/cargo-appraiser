//! Query types for cargo resolution lookups.

use cargo::core::{Package, Summary};

// Re-export DependencyTable from toml-parser
pub use toml_parser::DependencyTable;

/// Composite key for O(1) dependency lookup by (table, platform, name).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DependencyLookupKey {
    /// The dependency table (dependencies, dev-dependencies, build-dependencies)
    pub table: DependencyTable,
    /// Target platform if this is a platform-specific dependency (e.g., "cfg(windows)")
    pub platform: Option<String>,
    /// The crate name as declared in Cargo.toml
    pub name: String,
}

impl DependencyLookupKey {
    /// Create a new dependency lookup key.
    pub fn new(table: DependencyTable, platform: Option<String>, name: impl Into<String>) -> Self {
        Self {
            table,
            platform,
            name: name.into(),
        }
    }
}

/// Convert cargo's DepKind to DependencyTable.
pub fn dep_kind_to_table(kind: cargo::core::dependency::DepKind) -> DependencyTable {
    use cargo::core::dependency::DepKind;
    match kind {
        DepKind::Normal => DependencyTable::Dependencies,
        DepKind::Development => DependencyTable::DevDependencies,
        DepKind::Build => DependencyTable::BuildDependencies,
    }
}

/// Resolved dependency information from cargo.
///
/// Contains the installed package and version summaries for a dependency.
#[derive(Debug, Clone)]
pub struct ResolvedDependency {
    /// The resolved/installed Package (None if not installed)
    pub package: Option<Package>,
    /// All available versions from registry (sorted descending)
    pub available_versions: Vec<String>,
    /// Latest version compatible with version_req
    pub latest_matched_summary: Option<Summary>,
    /// Absolute latest version (may not be compatible with version_req)
    pub latest_summary: Option<Summary>,
}

impl ResolvedDependency {
    /// Returns true if the dependency is installed.
    pub fn is_installed(&self) -> bool {
        self.package.is_some()
    }

    /// Returns the installed version if available.
    pub fn installed_version(&self) -> Option<&semver::Version> {
        self.package.as_ref().map(|p| p.version())
    }

    /// Returns true if the installed version is the latest.
    pub fn is_latest(&self) -> bool {
        match (
            self.installed_version(),
            self.latest_matched_summary.as_ref(),
            self.latest_summary.as_ref(),
        ) {
            (Some(installed), Some(latest_matched), Some(latest)) => {
                installed == latest_matched.version() && latest_matched.version() == latest.version()
            }
            _ => false,
        }
    }

    /// Returns true if there's a compatible upgrade available.
    pub fn has_compatible_upgrade(&self) -> bool {
        match (self.installed_version(), self.latest_matched_summary.as_ref()) {
            (Some(installed), Some(latest_matched)) => installed != latest_matched.version(),
            _ => false,
        }
    }

    /// Returns true if the latest version is not compatible with the version requirement.
    pub fn has_incompatible_latest(&self) -> bool {
        match (
            self.latest_matched_summary.as_ref(),
            self.latest_summary.as_ref(),
        ) {
            (Some(latest_matched), Some(latest)) => {
                latest_matched.version() != latest.version()
            }
            _ => false,
        }
    }
}
