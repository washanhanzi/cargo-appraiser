//! Serializable entity types for cargo resolution results.
//!
//! These types use owned `String` values instead of cargo's `InternedString`
//! to avoid memory leaks in long-lived processes. They can be serialized
//! to JSON for IPC with worker processes.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::query::{DependencyLookupKey, ResolvedDependency};

/// Source kind for a dependency.
///
/// Indicates where a package comes from (registry, git, local path, etc.)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceKind {
    /// From crates.io
    CratesIo,
    /// From a git repository
    Git {
        /// Git reference (branch, tag, or rev)
        reference: Option<String>,
        /// Short commit hash (7 chars)
        short_commit: Option<String>,
        /// Full commit hash
        full_commit: Option<String>,
    },
    /// From a local path
    Path,
    /// From a directory
    Directory,
    /// From a custom registry
    Registry {
        /// Registry name
        name: String,
    },
}

impl Default for SourceKind {
    fn default() -> Self {
        SourceKind::CratesIo
    }
}

impl SourceKind {
    /// Returns true if this is a git source.
    pub fn is_git(&self) -> bool {
        matches!(self, SourceKind::Git { .. })
    }

    /// Returns true if this is a path source.
    pub fn is_path(&self) -> bool {
        matches!(self, SourceKind::Path)
    }

    /// Returns true if this is a directory source.
    pub fn is_directory(&self) -> bool {
        matches!(self, SourceKind::Directory)
    }

    /// Returns the git reference if this is a git source.
    pub fn git_reference(&self) -> Option<&str> {
        match self {
            SourceKind::Git { reference, .. } => reference.as_deref(),
            _ => None,
        }
    }

    /// Returns the short commit hash if this is a git source.
    pub fn short_commit(&self) -> Option<&str> {
        match self {
            SourceKind::Git { short_commit, .. } => short_commit.as_deref(),
            _ => None,
        }
    }

    /// Returns the full commit hash if this is a git source.
    pub fn full_commit(&self) -> Option<&str> {
        match self {
            SourceKind::Git { full_commit, .. } => full_commit.as_deref(),
            _ => None,
        }
    }
}

/// Feature map: feature name -> list of enabled features/dependencies.
pub type FeatureMap = HashMap<String, Vec<String>>;

/// Resolved package information.
///
/// Contains the essential data extracted from cargo's `Package` type.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResolvedPackage {
    /// Package version (e.g., "1.0.0")
    pub version: String,
    /// Source kind (registry, git, path, etc.)
    pub source: SourceKind,
    /// Available features and what they enable
    pub features: FeatureMap,
}

impl ResolvedPackage {
    /// Parse the version string into a semver::Version.
    pub fn semver_version(&self) -> Option<semver::Version> {
        semver::Version::parse(&self.version).ok()
    }
}

/// Version summary information.
///
/// Contains just the version string, extracted from cargo's `Summary` type.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VersionSummary {
    /// Version string (e.g., "1.0.0")
    pub version: String,
}

impl VersionSummary {
    /// Create a new version summary.
    pub fn new(version: impl Into<String>) -> Self {
        Self {
            version: version.into(),
        }
    }

    /// Parse the version string into a semver::Version.
    pub fn semver_version(&self) -> Option<semver::Version> {
        semver::Version::parse(&self.version).ok()
    }
}

/// Workspace member information for hover functionality.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceMember {
    /// Package name
    pub name: String,
    /// Path to the member's Cargo.toml
    pub manifest_path: PathBuf,
}

/// Serializable cargo index for IPC between LSP and worker subprocess.
///
/// This is the complete resolution result that can be serialized to JSON
/// and sent from the worker process to the LSP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableCargoIndex {
    /// The root manifest path
    pub root_manifest: PathBuf,
    /// Member manifest paths (for workspaces)
    pub member_manifests: Vec<PathBuf>,
    /// Workspace members (name and manifest path for hover)
    pub members: Vec<WorkspaceMember>,
    /// Resolved dependencies as (key, value) pairs for JSON serialization.
    /// JSON requires map keys to be strings, so we use a Vec of tuples instead of HashMap.
    pub dependencies: Vec<(DependencyLookupKey, ResolvedDependency)>,
}
