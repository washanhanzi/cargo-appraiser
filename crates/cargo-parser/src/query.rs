//! Query types for cargo resolution lookups.

use serde::{Deserialize, Serialize};

use crate::entity::{ResolvedPackage, SourceKind, VersionSummary};

// Re-export DependencyTable from toml-parser
pub use toml_parser::DependencyTable;

/// Composite key for O(1) dependency lookup by (table, platform, name).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
/// Uses owned types (String) instead of cargo's interned types to avoid
/// memory leaks in long-lived processes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResolvedDependency {
    /// The resolved/installed package info (None if not resolved)
    pub package: Option<ResolvedPackage>,
    /// All available versions from registry (sorted descending)
    pub available_versions: Vec<String>,
    /// Latest version compatible with version_req
    pub latest_matched_version: Option<VersionSummary>,
    /// Absolute latest version (may not be compatible with version_req)
    pub latest_version: Option<VersionSummary>,
}

impl ResolvedDependency {
    /// Returns true if the dependency is installed.
    pub fn is_installed(&self) -> bool {
        self.package.is_some()
    }

    /// Returns the installed version if available.
    pub fn installed_version(&self) -> Option<semver::Version> {
        self.package.as_ref().and_then(|p| p.semver_version())
    }

    /// Returns the installed version string if available.
    pub fn installed_version_str(&self) -> Option<&str> {
        self.package.as_ref().map(|p| p.version.as_str())
    }

    /// Returns the source kind if the package is installed.
    pub fn source_kind(&self) -> Option<&SourceKind> {
        self.package.as_ref().map(|p| &p.source)
    }

    /// Returns the features map if the package is installed.
    pub fn features(&self) -> Option<&crate::entity::FeatureMap> {
        self.package.as_ref().map(|p| &p.features)
    }

    /// Returns true if the installed version is the latest.
    pub fn is_latest(&self) -> bool {
        match (
            self.installed_version(),
            self.latest_matched_version
                .as_ref()
                .and_then(|s| s.semver_version()),
            self.latest_version
                .as_ref()
                .and_then(|s| s.semver_version()),
        ) {
            (Some(installed), Some(latest_matched), Some(latest)) => {
                installed == latest_matched && latest_matched == latest
            }
            _ => false,
        }
    }

    /// Returns true if there's a compatible upgrade available.
    pub fn has_compatible_upgrade(&self) -> bool {
        match (
            self.installed_version(),
            self.latest_matched_version
                .as_ref()
                .and_then(|s| s.semver_version()),
        ) {
            (Some(installed), Some(latest_matched)) => installed != latest_matched,
            _ => false,
        }
    }

    /// Returns true if the latest version is not compatible with the version requirement.
    pub fn has_incompatible_latest(&self) -> bool {
        match (
            self.latest_matched_version
                .as_ref()
                .and_then(|s| s.semver_version()),
            self.latest_version
                .as_ref()
                .and_then(|s| s.semver_version()),
        ) {
            (Some(latest_matched), Some(latest)) => latest_matched != latest,
            _ => false,
        }
    }
}
